# The eBPF Dataplane

**K8s analogue:** Cilium (eBPF datapath, kube-proxy replacement), Calico eBPF mode,
kube-proxy iptables/IPVS/nftables modes.

## Why eBPF

| Approach | Rule update cost | Per-packet cost | Notes |
|---|---|---|---|
| iptables | O(n) – whole table rewrite via `iptables-restore`; at 10k+ services takes seconds–minutes | O(n) linear chain walk for first packet of a connection (service lookup), then conntrack | kube-proxy default for a decade |
| IPVS | O(1) per service | O(1) hash lookup | Still relies on iptables for some features, conntrack |
| nftables (kube-proxy nftables mode, GA 1.33) | Incremental, sets/maps with O(1) lookups | O(1)-ish with verdict maps | Much better than iptables; still netfilter + conntrack |
| eBPF | O(1) map updates, atomic | O(1) map lookups; can bypass netfilter and conntrack | Requires modern kernel + BTF |

## Programming model in Rust: Aya

- **Aya** (`aya`, `aya-ebpf`): pure-Rust eBPF library – both the userspace loader *and*
  the kernel-side programs are written in Rust (compiled with the BPF target
  `bpfel-unknown-none` via `bpf-linker`). No libbpf, no clang/C toolchain required.
- Supports maps of all common types, TC (clsact + TCX), XDP, cgroup hooks,
  kprobes/tracepoints, LSM, ring buffers and map pinning in bpffs. The *loader* handles
  CO-RE relocations for objects that carry them. But **emitting** CO-RE relocations from
  Rust BPF programs is still experimental (a `bpf-linker` feature; a compiler-level
  `#[btf_preserve_access]` is only a pre-RFC). So Rustwain's programs must not read
  kernel structs. They work only on stable UAPI contexts (`__sk_buff`, `xdp_md`,
  `bpf_sock_addr`) and helpers, which also means they **need no kernel BTF**.
- Building the programs needs a **nightly** toolchain (`-Z build-std=core` for
  `bpfel-unknown-none`) and `bpf-linker`. Isolated to the `rustwain-bpf` crate (see
  [crate layout](../13-implementation/01-crate-layout.md#toolchains)).
- Alternatives: `libbpf-rs` (Rust bindings to libbpf, programs in C) – more mature
  verifier-friendly C, but breaks the Rust-only rule.
- Decision `[decided]`: **Aya**, programs in Rust. Risk: the BPF verifier is picky and
  Rust codegen can produce verifier-unfriendly patterns (bounded loops, stack size 512 B,
  complexity limits). Mitigation: keep programs small, use `#[inline(always)]`, test-load
  every program in CI against the kernel matrix (verifier behaviour differs by kernel).

## Hook points

| Hook | Attach | Kernel | Used for |
|---|---|---|---|
| `cgroup/connect4`, `connect6` | root cgroup (or rustwain.slice) | 4.17 | Socket-level service LB: rewrite dst at connect() |
| `cgroup/sendmsg4/6`, `recvmsg4/6` | cgroup | 4.18 / 5.2 | UDP service LB (DNS!) |
| `cgroup/getpeername4/6`, `getsockname` | cgroup | 5.8 | Make apps see the service VIP as peer |
| `cgroup/sock_create`, `post_bind` | cgroup | 4.10+ | Port restrictions, socket marking |
| `cgroup_skb/ingress,egress` | per pod cgroup | 4.10 | Simple per-pod accounting/filter (weaker than TC) |
| **TC** ingress/egress (clsact or **TCX** 6.6+) | host-side pod devs, physical NIC, tunnel dev | 4.1 / 6.6 | Policy, routing, DNAT for non-socket paths, encap decisions, MSS clamp, EDT |
| **netkit** primary/peer programs | netkit devices | 6.7 | Pod egress fast path |
| **XDP** | physical NIC | 4.8 | NodePort/LoadBalancer fast path, DDoS drop, Maglev LB before skb allocation |
| `sk_lookup` | netns | 5.9 | Steer incoming connections to sockets (e.g. transparent proxying for L7 policy) |
| `sockops` + `sk_msg` | cgroup | 4.13 / 4.17 | Socket-to-socket redirection for same-node (sockmap acceleration) – mixed results; `[open]` |
| LSM (BPF-LSM) | global | 5.7 | Optional security hooks |
| tracepoints / fentry | global | | Observability (deferred) |

TCX (6.6+) adds multi-program attach with ordering and links (safe ownership semantics) –
avoids clobbering other tools' TC programs. Use when present; fall back to clsact with
priority/handle discipline.

## Maps (state)

| Map | Type | Key → Value | Size driver |
|---|---|---|---|
| `endpoints` | hash | pod IP → {ifindex, identity, node, flags} | pods on node (local) + all pods? (see below) |
| `ipcache` | LPM trie | CIDR/IP → {identity, tunnel endpoint} | all pod IPs in cluster (or blocks) |
| `services` | hash | {VIP, port, proto, scope} → {backend_count, rev_nat_id, flags, algorithm} | services × ports |
| `backends` | hash/array | backend_id → {IP, port, zone, weight} | endpoints cluster-wide |
| `service_slots` | hash / array-of-maps | {service, slot} → backend_id (or Maglev table) | services × backends |
| `rev_nat` | hash | rev_nat_id → {VIP, port} | services |
| `policy` | hash per endpoint (map-in-map) or global | {src identity, dst port, proto, dir} → verdict | identities × rules |
| `ct` (own conntrack) | LRU hash | 5-tuple → state | connections (only where needed) |
| `nat` | LRU hash | for SNAT/masquerade (egress to outside) | external connections |
| `affinity` | LRU hash | {client, service} → backend | session affinity users |
| `metrics` | per-CPU array | reason → counters | fixed |
| `events` | ring buffer | drop/trace events | fixed |

### Map sizing and memory
Cluster-scale maps (ipcache, services, backends) exist on every node: at 150k pods and
10k services with 100k backends, this is tens of MB per node in BPF maps – acceptable but
not free. Techniques:
- Use **per-node-block entries** in ipcache (one /64 per node) instead of per-pod entries
  when identities are carried in packets (Geneve option) → O(nodes) not O(pods).
- Only program services actually reachable by local pods' policies? (`[open]` – lazy
  service programming, cf. Cilium's "service mesh scale" efforts.)
- Preallocated vs `BPF_F_NO_PREALLOC` hash maps: prealloc gives predictable latency but
  memory upfront.

## Packet walk: pod A (node 1) → Service S → pod B (node 2), TCP

1. **connect()** in pod A → `cgroup/connect6` program:
   - lookup `services[{S_VIP, 443, TCP}]` → select backend (random or Maglev by 5-tuple
     hash; topology preference) → rewrite socket dst to B_IP:8443.
   - store `{socket cookie → rev_nat_id}` for getpeername.
   - No packet-level NAT, **no conntrack entry needed** for the service translation.
2. SYN leaves pod A via netkit peer → netkit program (pod egress):
   - lookup `ipcache[B_IP]` → identity B, tunnel endpoint node 2.
   - **egress policy** check for A's identity → (B identity, 8443/TCP). Policy verdict cache
     in an LRU to avoid re-evaluating per packet (flow-level decision: only SYN does full
     lookup; established flows use own `ct` map entry).
   - Forward: native routing → `bpf_redirect_neigh(phys_if)` with FIB lookup; overlay →
     set tunnel key + Geneve option (src identity A) → redirect to `rwgnv0`.
3. Node 2, physical NIC → (tunnel decap) → TC ingress on tunnel dev / phys:
   - extract src identity from Geneve option (or ipcache lookup of src IP in native mode).
   - lookup `endpoints[B_IP]` → ifindex → **ingress policy** for B (src identity A,
     8443/TCP).
   - `bpf_redirect_peer(ifindex)` straight into pod B's netns.
4. Reply from B → reverse path; policy uses the `ct` entry (established) to allow return
   traffic without rule lookups.

Same node: step 2 redirects directly to B's device (`bpf_redirect_peer`) – never touches
the host stack.

## Non-socket paths that still need packet DNAT
- **MicroVM pods** (guest kernel's sockets don't run our cgroup hooks) → TC program on the
  TAP host side does per-packet DNAT + `ct` + reverse NAT.
- **hostNetwork pods** – they *are* in a cgroup, so socket LB works if hooks attach at the
  root cgroup.
- **External traffic** to NodePort/LoadBalancer → XDP/TC on physical NIC: DNAT (or DSR).
- Pods using **raw sockets / non-connect UDP patterns** – sendmsg hooks cover UDP; raw is
  rare.

## Packet walk: external client → NodePort with DSR

Setup: Service `web`, `type: NodePort`, port 30443/TCP → targetPort 8443,
`externalPolicy: Cluster`, `algorithm: Maglev`, `dsr: true`. Client C (198.51.100.7)
outside the cluster. Node N1 (10.0.0.1) receives the packet; the selected backend B
(fd00:10:42:2::17 / 10.42.2.17) runs on node N2 (10.0.0.2). Native routing underlay,
IPv4 shown for readability. DSR encapsulation: **Geneve with a Rustwain option**
carrying the original frontend `{N1_IP, 30443}` (the alternative IPv4 IP-option /
IPv6 destination-option encodings are listed below).

```
 C (198.51.100.7)          N1 phys NIC (XDP)            N2 phys NIC (TC ingress)   B (pod on N2)    N2 netkit (pod egress)
   │                              │                              │                     │                   │
   │ SYN 198.51.100.7:51000       │                              │                     │                   │
   │  → 10.0.0.1:30443            │                              │                     │                   │
   │─────────────────────────────▶│                              │                     │                   │
   │                              │ 1 parse eth/ip/tcp           │                     │                   │
   │                              │ 2 services[{10.0.0.1,30443,  │                     │                   │
   │                              │    TCP, external}] hit       │                     │                   │
   │                              │ 3 flows LRU miss (SYN)       │                     │                   │
   │                              │   → maglev[svc][hash5 % M]   │                     │                   │
   │                              │   → backend B@N2             │                     │                   │
   │                              │   flows[5-tuple] := B        │                     │                   │
   │                              │ 4 DNAT inner dst → B:8443    │                     │                   │
   │                              │   MSS clamp SYN option to    │                     │                   │
   │                              │   underlay_mtu − 40 − 58     │                     │                   │
   │                              │ 5 push outer IP/UDP/Geneve   │                     │                   │
   │                              │   (+opt {10.0.0.1,30443})    │                     │                   │
   │                              │   outer 10.0.0.1 → 10.0.0.2  │                     │                   │
   │                              │ 6 XDP_TX (same NIC) or       │                     │                   │
   │                              │   XDP_REDIRECT (other NIC)   │                     │                   │
   │                              │─────────────────────────────▶│                     │                   │
   │                              │                              │ 7 decap; read opt   │                     │
   │                              │                              │ 8 ct_dsr[{C:51000,  │                     │
   │                              │                              │    B:8443,TCP}] :=  │                     │
   │                              │                              │    {10.0.0.1,30443} │                     │
   │                              │                              │ 9 ingress policy B: │                     │
   │                              │                              │   src = world       │                     │
   │                              │                              │   identity, 8443 ✔  │                     │
   │                              │                              │10 redirect_peer(B)─▶│                     │
   │                              │                              │                     │ SYN-ACK           │
   │                              │                              │                     │ B:8443 → C:51000  │
   │                              │                              │                     │──────────────────▶│
   │                              │                              │                     │                   │11 ct_dsr hit →
   │                              │                              │                     │                   │   SNAT src to
   │                              │                              │                     │                   │   10.0.0.1:30443
   │                              │                              │                     │                   │12 FIB lookup →
   │                              │                              │                     │                   │   redirect_neigh
   │◀────────────────────────────────────────── SYN-ACK 10.0.0.1:30443 → 198.51.100.7:51000 ───────────────│   (out N2 phys,
   │                              │                              │                     │                   │    not via N1)
   │ ACK / data → 10.0.0.1:30443  │                              │                     │                   │
   │─────────────────────────────▶│ flows LRU hit → B; encap,    │                     │                   │
   │                              │ (no option needed after SYN  │                     │                   │
   │                              │  – N2 already has ct_dsr)    │                     │                   │
```

Step notes:

- **Step 1.** **XDP runs before skb allocation**, so a NodePort hit that leaves the node again
   (remote backend) never builds an skb on N1. Non-matching packets return `XDP_PASS`
   with a single hash lookup of overhead.
- **Step 2.** **Frontend key includes scope**: `external` frontends match only on the physical
   NIC hook; the same port on a pod-facing hook is not a NodePort. NodePort frontends are
   programmed for every node IP (and optionally only for specific interfaces –
   `nodePortAddresses`).
- **Step 3.** **Backend choice must be stable across packets and across nodes.** Maglev on the
   5-tuple gives every node the same answer for the same flow, so ECMP re-hashing an
   established flow to N3 still lands on B. The small `flows` LRU on N1 pins flows
   through backend-set changes (Maglev's minimal disruption is not zero disruption).
   Without it, scaling `web` from 10 to 11 replicas would reset ~1/11 of established
   external connections.
- **Step 4.** **MSS clamping is mandatory with encapsulated DSR.** C's SYN advertises an MSS
   derived from its own 1500-byte MTU; inner packets of that size plus outer
   IP/UDP/Geneve(+option) headers (~58 bytes) exceed the underlay MTU. PMTU discovery
   can't fix it – ICMP "frag needed" from the underlay would go to N1's outer source,
   not to C. Rewriting MSS in the SYN (and updating the TCP checksum with
   `bpf_l4_csum_replace`) is the only reliable fix. UDP NodePorts with DSR therefore
   require headroom or fall back to SNAT mode (configurable per service).
- **Step 5.** **Encapsulation choice:**

   | Encoding | Overhead | Works through | Notes |
   |---|---|---|---|
   | Geneve + option (default) | ~58 B | Anything that carries UDP | Reuses the overlay device; option also carries src identity |
   | IPv4 IP option | 8 B | Routers that don't drop/slow-path IP options – many do | Cilium supports; often punted to CPU by hardware routers |
   | IPv6 destination option ext. header | 8 B | IPv6 underlays that pass ext. headers | Cleanest on v6-only underlays |

- **Step 8.** **N2 keeps reverse-NAT state, N1 keeps none for the reply path.** `ct_dsr` is an LRU
   keyed by the backend-side tuple; entries are refreshed by inbound packets and expire
   on FIN/RST + timeout. Its size bounds the number of concurrent DSR flows per node
   (default 256k entries ≈ 30 MiB); LRU eviction of a live entry makes the next reply
   leave unrewritten (sourced from B's pod IP, which C never matches to its connection, so the flow stalls until C times out) – an `events` ring-buffer
   drop reason `DsrStateEvicted` makes this visible.
- **Step 11.** **The reply leaves N2 with source 10.0.0.1** – an address that isn't N2's. This is
  the whole point of DSR and also its main deployment constraint: **cloud anti-spoofing
   must be off** (AWS ENI source/dest check, GCP `canIpForward`, Azure IP forwarding)
   and upstream routers must not run strict uRPF on this path. The agent probes this at
   join time by sending a crafted packet to a peer node with a foreign source and
   reports `node.rustwain.io/dsr-capable` as an attribute; services with `dsr: true`
   fall back to SNAT on nodes without it. Tracked as `PP-R2`.

**Local-backend shortcut:** if the Maglev pick is on N1 itself, step 5–6 become DNAT +
`bpf_redirect_peer` into the pod, and the reply's reverse NAT happens in N1's netkit
program – no encapsulation, same `ct_dsr` map.

**Health and `Local` policy:** with `externalPolicy: Local`, N1's backend table only
contains local endpoints, DSR is unnecessary and step 5 never happens. With `Cluster` +
DSR the client IP is preserved *and* load is spread – the combination Kubernetes can only
reach with Cilium-specific configuration (`PP-6C`).

## Packet walk: microVM pod → Service

Setup: pod A runs in a microVM (`isolation: microvm`, see
[KVM and microVMs](../03-virtualization/02-kvm-and-microvms.md)) on N1. The VMM
(cloud-hypervisor class) exposes a virtio-net device backed by a host TAP `rwt3f9a`
with vhost-net. The guest kernel has its own TCP stack, so **cgroup socket hooks never
see A's `connect()`** – service translation happens per packet on the TAP. A connects to
Service `api` VIP fd00:10:96::a:443 → backend B (pod on N2, fd00:10:42:2::21:8443).

```
 guest app     guest kernel      vhost-net / TAP      N1 TC ingress on rwt3f9a      N1 egress          N2 → B
   │                │            (host side)                  │                     │                 │
   │ connect(VIP)   │                 │                       │                     │                 │
   │───────────────▶│ ND for gateway  │                       │                     │                 │
   │                │ fe80::1 ───────▶│──────────────────────▶│ 0 NDP/ARP responder │                 │
   │                │◀───────────────────────────── NA (gw MAC) ◀ in BPF (no host   │                 │
   │                │                 │                       │   neighbour entry)  │                 │
   │                │ SYN A→VIP:443   │                       │                     │                 │
   │                │ (GSO/csum       │                       │                     │                 │
   │                │  partial ok)    │                       │                     │                 │
   │                │────────────────▶│ vhost kthread writes  │                     │                 │
   │                │                 │ skb → rwt3f9a rx ────▶│ 1 anti-spoof: src   │                 │
   │                │                 │                       │   MAC/IP == A's     │                 │
   │                │                 │                       │   allocation, else  │                 │
   │                │                 │                       │   drop SpoofedSrc   │                 │
   │                │                 │                       │ 2 services[VIP:443] │                 │
   │                │                 │                       │ 3 ct[A:p→VIP:443]   │                 │
   │                │                 │                       │   miss (SYN): pick  │                 │
   │                │                 │                       │   backend B, insert │                 │
   │                │                 │                       │   ct{→B, rev_nat}   │                 │
   │                │                 │                       │ 4 DNAT dst → B:8443 │                 │
   │                │                 │                       │   (l3/l4 csum diff; │                 │
   │                │                 │                       │   partial csum safe)│                 │
   │                │                 │                       │ 5 egress policy for │                 │
   │                │                 │                       │   A's identity      │                 │
   │                │                 │                       │ 6 ipcache[B] → N2   │                 │
   │                │                 │                       │   encap/route,      │                 │
   │                │                 │                       │   bpf_check_mtu     │                 │
   │                │                 │                       │   (GSO-aware) ─────▶│ redirect_neigh  │
   │                │                 │                       │                     │ / tunnel ──────▶│ (normal pod
   │                │                 │                       │                     │                 │  ingress walk)
   │                │                 │                       │                     │                 │
   │                │                 │      reply B:8443 → A:p arrives on N1 (tunnel/phys TC ingress)│
   │                │                 │                       │ 7 endpoints[A] →    │                 │
   │                │                 │                       │   ifindex rwt3f9a,  │                 │
   │                │                 │                       │   kind=Tap          │                 │
   │                │                 │                       │ 8 ct hit → reverse  │                 │
   │                │                 │                       │   NAT src B:8443 →  │                 │
   │                │                 │                       │   VIP:443           │                 │
   │                │                 │                       │ 9 bpf_redirect(     │                 │
   │                │                 │◀──────────────────────│   rwt3f9a, egress)  │                 │
   │                │◀────────────────│ vhost → guest rx ring │   (not _peer: TAP   │                 │
   │◀───────────────│ SYN-ACK from VIP│                       │   has no peer netns)│                 │
```

What differs from the container walk, and why:

| Concern | Container pod | MicroVM pod |
|---|---|---|
| Service translation | Socket LB at `connect()`, no per-packet NAT | **Per-packet DNAT + `ct` entry** on TAP ingress, reverse NAT on delivery |
| Conntrack state | None for east-west services | One `ct` entry per flow – size the map for VM density (`PP-R4`) |
| Gateway resolution | netkit is L3 (no ARP/ND needed) | Guest does ARP/ND; BPF answers for the gateway address so no host neighbour/proxy-ARP sysctls are needed |
| Source trust | Container shares host kernel; spoofing needs `CAP_NET_RAW` (dropped by default) | Guest is root in its own kernel – **assume every packet can be forged**; enforce MAC+IP per TAP |
| Delivery into the pod | `bpf_redirect_peer` (netns switch, no backlog queue) | `bpf_redirect` to TAP egress → vhost-net queue |
| Offloads | Kernel stack handles | Guest TSO produces up to 64 KiB GSO skbs; checksum may be `CHECKSUM_PARTIAL`. NAT must use incremental csum helpers; MTU checks must be GSO-aware (`bpf_check_mtu` with `BPF_MTU_CHK_SEGS`) |
| DNS | Socket LB on UDP sendmsg to the node-local resolver | Guest resolv.conf points to the node-local resolver IP directly (no VIP), so DNS skips `ct` entirely |
| Policy identity | From netkit ifindex → endpoint | From TAP ifindex → endpoint (same `endpoints` map, `kind=Tap`) |

Failure note: `ct` entries for microVM flows are on the **critical path of correctness**
(losing one mid-flow sends replies with the backend's address, which the guest drops).
The `ct` map for TAP endpoints is therefore a separate, preallocated hash (not an LRU
shared with best-effort maps): when full, *new* flows are dropped with reason `CtFull`
(the guest retries the SYN) instead of silently breaking established ones, and a
non-zero `CtFull` counter is surfaced as a node condition.

## Masquerade / egress SNAT
Traffic from pods to outside the cluster CIDRs → SNAT to node IP. BPF-based NAT with
port allocation (`nat` map) instead of iptables MASQUERADE; avoids conntrack table
pressure (the `nf_conntrack: table full, dropping packet` classic). IPv6 GUA pods may
skip NAT entirely.

## Conntrack strategy
- East-west service traffic: none (socket LB).
- Policy for stateful return traffic: our own `ct` LRU map, only for flows that need it.
- Kernel netfilter conntrack: **bypassed** for pod traffic where possible – but the
  kernel still creates conntrack entries if netfilter hooks are active (any iptables rule
  in the host, e.g. Docker). We can mark packets `NOTRACK`-equivalent only via
  netfilter… so we document: with the eBPF dataplane, pod traffic redirected via
  `bpf_redirect_*` from TC skips the netfilter hooks of the host stack entirely
  (that's the main win).

## Loading, upgrade and persistence
- Programs and maps are **pinned** under `/sys/fs/bpf/rustwain/`. On agent restart the
  new agent opens pinned maps (reusing state – conntrack, NAT entries, affinity) and
  atomically replaces programs (TCX link update or `bpf_link_update`) – **zero packet
  loss across agent upgrades**.
- Map schema changes across versions: versioned map names (`services_v2`); migration code
  copies entries, then swaps programs. Pain point in every eBPF project – design for it
  from the start.

## Fallback: nftables backend `[later]`

The kernel floor (5.15) already includes every hook the core dataplane uses, and the
core programs need no kernel BTF, so a missing BTF is **not** a reason to fall back. The
fallback is for hosts where BPF is unusable: no `CAP_BPF`/`CAP_SYS_ADMIN` (agent inside a
restricted container), a security policy that forbids BPF, or an operator's choice. That
makes it a M7-or-later item, not part of the first networking milestone:
- `table inet rustwain` with sets and verdict maps:
  - Services: `map svc_vip_port : verdict` → `jump svc_<id>` chains with `numgen random
    mod N vmap` for backend selection → `dnat`.
  - Policy: sets of IPs per identity, per-pod chains.
- Updates via nftables netlink transactions (atomic batch) – incremental, no full
  rewrites. We write a Rust nftables netlink encoder (the `rustables`/`nftnl` crates
  exist but are thin; JSON-via-`nft` binary breaks the no-shell-out rule).
- Uses kernel conntrack (unavoidable with netfilter NAT).
- Feature parity target: services, policy L3/L4, masquerade, host ports. Not: socket-LB,
  EDT bandwidth, identity-in-packet.

## Testing BPF code
- Unit tests: `BPF_PROG_TEST_RUN` (`bpf_prog_test_run_opts`) lets us run TC/XDP programs
  against crafted packets in the kernel without real traffic – used heavily in CI.
- Verifier tests on the kernel matrix (5.15, 6.1, 6.6, 6.12, latest) in VMs.
- Packet-level integration tests with netns topologies built by test harness.
- Fuzz the userspace map-programming logic (property: map state == model of API state).

## Open questions

- Per-endpoint policy maps (map-in-map, Cilium style) vs a global policy map keyed by
  {dst endpoint, src identity, port}: memory vs update cost trade-off.
- sockmap acceleration for same-node: measured gains are workload-dependent; skip initially.
- How much of XDP to use initially (NodePort fast path) – XDP needs driver support for best
  performance (native mode); generic XDP is slower than TC.

## Build checklist

- [ ] `rustwain-bpf` crate (kernel programs, `no_std`) + `rustwain-dataplane` loader.
- [ ] Socket-LB programs (connect, sendmsg/recvmsg, getpeername).
- [ ] TC/netkit programs: routing, policy, encap, MSS clamp, EDT.
- [ ] Pinned-map upgrade path with versioned schemas.
- [ ] `BPF_PROG_TEST_RUN` test suite.
- [ ] nftables backend with netlink encoder.
