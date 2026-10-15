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
- Supports BTF/CO-RE relocations for kernel struct access, maps of all common types,
  TC (clsact + TCX), XDP, cgroup hooks, kprobes/tracepoints, LSM, ring buffers, map
  pinning in bpffs.
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

## Fallback: nftables backend

For nodes without BTF / sufficient BPF features (or by choice):
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
