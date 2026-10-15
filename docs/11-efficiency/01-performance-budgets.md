# Performance Budgets

Concrete, testable numbers. Each budget has: a metric, a target, how it's measured, and
the CI test that enforces it. Budgets start as **targets** and become **gates** once the
component exists and we have a baseline. Numbers for Kubernetes are typical published or
commonly observed values, for orientation only – always re-measure on the same hardware.

## Node-level resource overhead

| Component | Metric | Target | Kubernetes reference | Enforced by |
|---|---|---|---|---|
| Agent idle (0 pods) | RSS | < 25 MB | kubelet ~50–100 MB + containerd ~30–60 MB + kube-proxy ~20–40 MB + CNI agent 100–300 MB (Cilium) | `bench/agent_idle_rss` |
| Agent with 100 pods | RSS | < 40 MB | kubelet grows with pods (~100–200 MB) | `bench/agent_100_pods_rss` |
| Agent with 100 pods steady state | CPU | < 1% of one core avg | kubelet+runtime 2–5% (PLEG, stats, probes) | `bench/agent_steady_cpu` |
| Supervisor per pod | RSS | < 1 MB (PSS) | containerd-shim ~3–8 MB; pause ~0.3 MB | `bench/supervisor_rss` |
| eBPF maps (1k services, 10k endpoints) | kernel memory | < 30 MB | Cilium similar | `bench/bpf_map_mem` |
| Static binary size | on disk | < 60 MB `rustwain` | k8s node binaries total > 300 MB | CI size check |

Measurement: PSS (proportional set size) via `/proc/<pid>/smaps_rollup` for shared pages;
cgroup `memory.current` for `rustwain-system` cgroup totals.

## Latency

| Operation | Target p50 / p99 | Measured from → to |
|---|---|---|
| Pod start (warm image, process class, single container) | 150 ms / 400 ms | agent receives pod → container `execve` |
| Pod start end-to-end via API (warm image) | 400 ms / 1 s | `apply` returns → pod Ready=True observed by client (no probes) |
| Pod start, microvm class (template snapshot restore) | 200 ms / 500 ms | agent receives pod → container running in guest |
| Pod start, microvm cold boot | 400 ms / 1 s | same |
| Pod network setup | 3 ms / 10 ms | netns ready → interface + routes + BPF ready |
| Container stop (graceful app) | grace-bound | – |
| Service endpoint change → all nodes programmed (1k nodes) | 200 ms / 1 s | endpoint write commit → last node map update |
| Readiness failure → removed locally | < 10 ms after probe result | local fast path |
| API write (3 servers, NVMe) | 5 ms / 25 ms | request → response |
| API consistent get (from cache) | 1 ms / 5 ms | |
| Watch event delivery | 5 ms / 50 ms | commit → client receives |
| Scheduling (simple pod, 5k nodes) | 1 ms / 10 ms per pod; ≥ 1,000 pods/s throughput | |
| DNS cluster lookup (node-local) | 50 µs / 200 µs | query → response at pod |
| Agent restart → fully reconciled (100 pods) | 1 s / 3 s | process start → Ready condition |

## Throughput / scale

| Dimension | Target | Kubernetes official SLO-scale reference |
|---|---|---|
| Nodes per cluster | 5,000 (stretch 10,000) | 5,000 |
| Pods per cluster | 150,000 (stretch 300,000) | 150,000 |
| Pods per node | 500 supported (hardware-bound) | 110 default |
| Services | 10,000 | 10,000 |
| Endpoints per service | 5,000 | ~5,000 (practical) |
| API writes/s sustained | 10,000 | etcd-bound, typically low thousands |
| Watchers per server | 50,000 | – |
| Pod churn (create+delete)/s cluster-wide | 1,000 | ~100s |

## Control-plane resources (5k nodes, 150k pods, 10k services)

| Server process | Target |
|---|---|
| RSS per server (store + API + watch hub) | < 8 GB |
| CPU at steady state | < 4 cores |
| CPU during full-cluster rollout (150k pods replaced over 30 min) | < 16 cores |
| Store size on disk | < 4 GB snapshot |

## Dataplane

| Metric | Target |
|---|---|
| Pod-to-pod TCP throughput (same node, netkit) | ≥ 95% of host loopback-equivalent path |
| Pod-to-pod TCP throughput (cross node, native routing) | ≥ 95% of host-to-host |
| Overlay (Geneve) throughput | ≥ 85% of host-to-host |
| WireGuard throughput per core | ≥ 3 Gbit/s on modern x86 core |
| Added latency per hop (RR test, netperf TCP_RR) | < 5 µs vs host network |
| Service connection setup overhead (socket-LB) | ~0 extra vs direct pod IP |

## How budgets are enforced
- `cargo xtask bench <suite>` runs benchmarks on a reference VM shape (fixed vCPU/memory,
  pinned kernel version) to reduce noise.
- Results stored as JSON artifacts; CI compares against the baseline with tolerances
  (e.g. +10% RSS or +20% latency fails the build).
- Microbenchmarks with `criterion` for hot paths (encoding, watch dispatch, policy
  compiler, scheduler filters).
- Macro benchmarks in VMs for startup latency and dataplane throughput (netperf/iperf3
  equivalents – we can write a small Rust load generator to avoid non-Rust tooling in CI,
  or accept standard tools in test VMs only).
- Scale tests with simulated nodes (fake agents in-process) for control-plane budgets.

## Open questions
- Reference hardware for dataplane benchmarks (needs real NICs for meaningful numbers;
  VMs with virtio-net distort results).

## Build checklist
- [ ] `xtask bench` framework with baseline comparison.
- [ ] Agent RSS/CPU benches.
- [ ] Pod start latency bench with phase breakdown (from `PodTimings`).
- [ ] Simulated-node control-plane scale harness.
