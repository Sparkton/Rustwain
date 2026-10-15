# The Network Model

**K8s analogue:** the Kubernetes network model (every pod gets an IP; pods reach each
other without NAT), CNI, kube-proxy, Services, NetworkPolicy.

## The contract every pod gets `[decided]`

1. **Every pod has its own IP address(es)** – IPv6, IPv4 or both. Containers in a pod share
   it (same netns).
2. **Any pod can reach any other pod by IP without NAT**, across nodes, subject only to
   network policy. The source IP the receiver sees is the sender pod's IP.
3. **Agents (and host-network processes) can reach every pod** on their node (for probes),
   subject to policy for non-probe traffic.
4. **Services** provide a stable virtual IP and DNS name that load-balances to ready pods.
5. **DNS** resolves service and pod names inside the cluster.
6. **Network policy** is identity-based and default-allow per tenant unless the tenant
   sets default-deny (`[open]`: default-deny for new tenants?).
7. **Egress** to outside the cluster is SNATed to the node IP by default (configurable:
   egress gateways, or routable pod IPs with no SNAT).

This is the Kubernetes model, intentionally – it's simple for applications (no port
mapping, every pod looks like a host) and it's what software expects.

## Design goals beyond Kubernetes

| Goal | How |
|---|---|
| O(1) service updates and lookups | eBPF hash maps, no iptables |
| No conntrack for east-west service traffic | Socket-level LB rewrites at `connect()` |
| Identity-based policy that scales | Numeric security identities in packet metadata / maps, not IP lists |
| IPv6-first | Single-stack v6 supported as first-class; dual-stack default |
| Encryption built-in | WireGuard between nodes, one flag |
| Fast pod network setup | netkit/veth via netlink, no CNI exec chain |
| Works on old-ish kernels | nftables fallback dataplane |
| Observable | Every drop has a reason code exported via a ring buffer `[later with observability]` |

## Components

```
 ┌───────────── control plane ──────────────┐
 │ IPAM controller   (node CIDR allocation) │
 │ endpoint controller (Service → EndpointSets) │
 │ identity controller (labels → numeric identities) │
 │ policy compiler     (policies → per-identity rules) │
 └────────────────────┬─────────────────────┘
                      │ watch
 ┌──────────── agent dataplane module ──────────────┐
 │ local IPAM      (pod IPs from node CIDR)         │
 │ link manager    (netkit/veth/tap, routes, MTU)   │
 │ BPF loader      (aya; programs + pinned maps)    │
 │ service syncer  (EndpointSets → LB maps)         │
 │ policy syncer   (identities+rules → policy maps) │
 │ tunnel/route mgr(VXLAN/Geneve/native/BGP/WG)     │
 │ DNS server      (node-local)                     │
 │ fallback: nftables backend                       │
 └──────────────────────────────────────────────────┘
```

## Routing modes

| Mode | How pods on different nodes reach each other | When |
|---|---|---|
| **Native routing (L3)** | Each node gets a pod CIDR; nodes have routes `podCIDR(n) via nodeIP(n)` | All nodes on one L2 segment, or the underlay can route pod CIDRs (BGP to ToR, cloud route tables) |
| **Overlay (VXLAN/Geneve)** | Encapsulate pod packets in UDP between node IPs | Default when underlay can't route pod IPs; works anywhere with UDP |
| **WireGuard** | Encrypted tunnel per node pair (one wg device, peers per node) | When encryption is required; doubles as overlay |
| **Cloud-native (ENI/VPC IPs)** | Pods get VPC IPs directly (AWS VPC CNI, Azure CNI) | Cloud plugins `[later]`; IP exhaustion trade-offs |

Default `[proposed]`: auto-detect – if all nodes share a subnet, native routing with direct
routes; otherwise Geneve overlay. Encryption via WireGuard opt-in (one setting).

## MTU

Overlay encapsulation reduces MTU: VXLAN/Geneve over IPv4 = 50 bytes overhead (more
with Geneve options; IPv6 underlay +20); WireGuard = 60 (v4) / 80 (v6). Get this wrong and
you get mysterious hangs on large transfers (PMTUD blackholes) – one of the most common
CNI support issues.

Rustwain: compute pod MTU = uplink MTU − encapsulation overhead, apply to pod interfaces
and routes, and **clamp TCP MSS** in the BPF program as a safety net. Detect jumbo frames
(9001 on AWS) and use them. Expose effective MTU in node status.

## Node-to-node and control-plane ports

| Port | Proto | Purpose |
|---|---|---|
| 6443 `[proposed]` | TCP | API (clients, agents) |
| 6444 | TCP | Raft peer traffic (servers only) |
| 6081 | UDP | Geneve (if overlay) |
| 8472 | UDP | VXLAN (if overlay, alternative) |
| 51871 | UDP | WireGuard (if enabled) |
| — | — | Agents need no inbound port with the dial-out tunnel design |

## What we deliberately don't do (initially)
- Multiple networks per pod (Multus-style) – `[later]` as "attachments", needed for VMs
  and NFV.
- L2 pod networking across nodes (stretch L2) – no.
- Network namespaces shared between pods – no.

## Open questions

- Default-deny for new tenants?
- Should Services be L4 only in core, with L7 (HTTP routing, retries) in the gateway layer?
  Lean yes.

## Build checklist

- [ ] Dataplane module trait with `ebpf` and `nftables` backends.
- [ ] Routing mode auto-detection.
- [ ] MTU calculation and MSS clamping.
- [ ] Port/firewall doctor checks.
