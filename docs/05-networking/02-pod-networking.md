# Pod Networking: Interfaces, Routing, Overlays

**K8s analogue:** CNI plugins (bridge, ptp, host-local IPAM, flannel, calico, cilium),
CNI spec ADD/DEL/CHECK.

## How CNI works (for contrast)

CNI is an exec-based protocol: the runtime runs plugin binaries with env vars
(`CNI_COMMAND=ADD`, `CNI_NETNS=/var/run/netns/x`, `CNI_IFNAME=eth0`) and JSON config on
stdin; the plugin configures the netns and prints a JSON result. Plugins chain (main →
portmap → bandwidth → tuning). Pros: simple, language-agnostic, huge ecosystem. Cons:
process spawn per plugin per pod, no long-lived state (needs a daemon anyway for
anything interesting), fragile error handling on DEL (leaked IPs are a classic), no
update operation, version skew between plugin binaries and daemons.

Rustwain does pod network setup **in-process** in the agent's dataplane module. A CNI
compatibility host `[open]` could run third-party CNI plugins for special cases (e.g.
SR-IOV, Multus-style secondary networks).

## Interface types

### veth pairs
- Classic: two virtual ethernet devices connected back-to-back; one end in pod netns
  (`eth0`), one on host (`rwh<id>`).
- Every packet crosses the host-side veth → host stack (routing) or is redirected by BPF.
- Overhead: the "veth → host → veth" path adds a softirq traversal and backlog queue
  hop; `bpf_redirect_peer` (5.10+) lets a TC program on the host's physical NIC ingress
  deliver directly into the pod's netns, skipping the host-side veth processing.

### netkit (6.7+)
- A device pair designed for BPF-programmable container networking (from the Cilium
  folks). The BPF program is attached to the device *itself* as its "xmit" logic, running
  in the pod's netns context on egress – packets leaving the pod can be redirected
  straight to the physical NIC without traversing the host stack, and there's no
  backlog queue / per-CPU softirq switch.
- Benchmarks show netkit gets pod networking within a few percent of host networking
  throughput/latency (veth typically costs more, especially for small packets).
- Modes: L3 (no ARP/MAC in pod – simpler) or L2.
- **Rustwain default when available**; veth fallback.

### TAP
- For microVMs: VMM reads/writes frames from a TAP fd; host side is a normal interface
  with BPF attached. vhost-net offload.

### ipvlan / macvlan
- Sub-interfaces of a physical NIC; pods appear directly on the underlay L2. Low
  overhead, but host↔pod traffic is awkward (macvlan can't talk to its parent), and
  policy enforcement on the host is bypassed for ipvlan L2. Not default; maybe for
  "attachments".

## Per-pod setup sequence (netkit, L3 mode)

```
1. supervisor creates netns, reports fd
2. dataplane:
   a. ip = ipam.allocate(pod_uid)                       (local, no API call)
   b. create netkit pair: primary "rwh<idx>" in host ns, peer in pod netns
      (RTM_NEWLINK with IFLA_NETKIT_* attrs; peer netns via IFLA_NET_NS_FD)
   c. in pod netns (via a setns'd helper thread or netlink socket opened in that netns):
        - rename peer to eth0, set MTU, set up
        - addr add <ip>/128 (v6) or /32 (v4)  ← host-route style; no subnet in pod
        - route default via link-local gateway (fe80::1 / 169.254.1.1) dev eth0, onlink
        - sysctls: disable DAD for speed (accept_dad=0), ping_group_range
   d. on host: route <ip> dev rwh<idx> (or rely purely on BPF redirect + no route)
   e. attach BPF programs (netkit primary/peer hooks), insert endpoint into
      endpoint map {ip → ifindex, identity, mac, pod_uid}
3. report ready
```

Everything via netlink sockets; opening a netlink socket *inside* the pod netns once
(created by a thread that `setns`'d, then the fd is used from anywhere) avoids repeated
`setns` thread hopping.

Target: < 5 ms for the whole sequence (CNI chains typically take 50–500 ms).

## Addressing scheme

- Cluster pod CIDRs: e.g. IPv6 `fd00:10:42::/48` (ULA) or a GUA prefix; IPv4
  `10.42.0.0/16`.
- Per-node blocks: IPv6 /64 per node (plenty); IPv4 /24 per node by default (254 pods),
  **dynamically extended** with additional blocks when a node runs out (Calico-style
  block affinity) rather than a fixed per-node CIDR (Kubernetes' `podCIDR` is fixed per
  node, so a dense node can run out while the cluster has free space).
- See [IPAM and dual stack](08-ipam-and-dual-stack.md).

## Cross-node forwarding

### Native routing
- Each agent installs kernel routes for other nodes' blocks
  (`fd00:10:42:7::/64 via <node7-ip>`) – or does the lookup in BPF (a LPM trie map
  `pod_cidr → node_ip`) and uses `bpf_fib_lookup` + `bpf_redirect_neigh` to forward.
- With BGP (optional): agents announce their blocks to ToR routers so the underlay
  routes pod IPs. Implementation: embed a minimal BGP speaker in the agent `[later]`
  (there's no mature pure-Rust BGP daemon; candidates: `holo` routing suite (Rust),
  or writing a minimal iBGP announcer). Many on-prem users want this.

### Overlay (Geneve default, VXLAN alternative)
- Single collect-metadata ("external") tunnel device per node (`rwgnv0`); BPF sets the
  tunnel key (remote node IP, VNI, options) per packet with `bpf_skb_set_tunnel_key` –
  no per-peer devices, no FDB management.
- Geneve TLV options can carry the **source security identity** so the receiving node
  enforces policy without an ipcache lookup (Cilium does this with VXLAN VNI = identity).
- UDP source port derived from inner flow hash → ECMP/RSS spreading on the underlay.
- Checksum/GSO offloads: modern NICs offload VXLAN/Geneve segmentation (`tx-udp_tnl-segmentation`).

### WireGuard
- One `rwwg0` device; peers = other nodes (public key + endpoint + allowed IPs = their
  pod blocks). Keys rotated, distributed via Node objects (public keys only).
- Kernel WireGuard (5.6+); configuration via generic netlink (`wireguard-uapi` style –
  we write a thin netlink client).
- See [encryption and multicluster](09-encryption-and-multicluster.md).

## Host networking pods
`hostNetwork: true` pods share the host netns – used by system DaemonSets. They get the
node's identity for policy (`host` identity). Gated by security level.

## Host ports
`hostPort: 8080` on a pod container: DNAT from node IP:8080 to the pod. Implemented in the
BPF datapath (service-like map entry scoped to the node IP), not iptables `portmap`.

## Bandwidth limiting
Kubernetes' `bandwidth` CNI plugin uses TBF qdiscs (HTB/TBF) – CPU-expensive and lock
contended. Rustwain uses **EDT (Earliest Departure Time)**: BPF sets `skb->tstamp` per
packet based on the pod's rate; the `fq` qdisc on the physical NIC releases packets at
their departure time. Scales well (this is what Cilium's bandwidth manager does, from
Google's research). Egress limits primarily; ingress limiting is inherently lossy (drop).

## Secondary networks ("attachments") `[later]`
For VMs and NFV: pods can request additional interfaces attached to named networks
(VLAN on a physical NIC via macvlan/ipvlan, SR-IOV VF, bridge). Defined via a
`NetworkAttachment` object (like Multus' `NetworkAttachmentDefinition`). Policy
enforcement on these is limited and documented.

## Open questions

- Should the pod's default route use a link-local gateway (Calico/Cilium style) or a real
  gateway IP? Link-local is simpler and avoids wasting an address.
- BGP speaker: build or integrate?

## Build checklist

- [ ] Netlink link manager: netkit, veth, tap, geneve/vxlan (collect_md), wireguard.
- [ ] Per-pod setup sequence with timing instrumentation; < 5 ms target test.
- [ ] Native-routing and Geneve overlay modes with LPM trie.
- [ ] EDT bandwidth limiting.
- [ ] Host port support in BPF.
