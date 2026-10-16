# Testing Strategy

An orchestrator fails in the gaps between components, under faults, on kernels you didn't
test. The strategy layers tests from fast/deterministic to slow/realistic.

## Pyramid

| Layer | What | Where it runs | Speed |
|---|---|---|---|
| Unit | Pure logic: validation, conversions, scheduler plugins, policy compiler, IPAM, SSA merge | `cargo test` anywhere (macOS dev OK for pure crates) | ms |
| Property | Invariants over random inputs: SSA idempotence, conversion round-trips, pod state machine, scheduler never overcommits, IPAM never double-allocates | `cargo test` | s |
| Fuzz | Untrusted input parsers: tar unpacker, image manifests, DNS packets, API decoding, CEL expressions, netlink responses | `cargo fuzz` nightly CI | hours |
| Simulation | Whole control plane (+ fake agents) in deterministic single-threaded sim with fault injection and simulated time; linearizability checks | `cargo test --features sim` | s–min |
| BPF unit | `BPF_PROG_TEST_RUN` with crafted packets | Linux CI VMs, kernel matrix | s |
| Integration (privileged) | Real namespaces/cgroups/mounts/netlink on a real kernel | Linux VMs via `xtask vm-test` | min |
| Distro matrix | Same integration suite on Tier-1/2 distros | Nightly | hour |
| Cluster e2e | 3 servers + N agents in VMs; workloads, failures, upgrades | Nightly | hour |
| Conformance-style | Port of relevant Kubernetes e2e semantics (NetworkPolicy, Services, probes, Job policies) | Nightly | hour |
| Chaos | Random kills, partitions, disk faults, clock skew with invariant checkers | Nightly/weekly | hours |
| Scale | Simulated nodes (thousands of fake agents) against real servers | Weekly | hours |
| Performance | Budget benches vs baselines | Per-merge (micro) / nightly (macro) | min–hour |

## Key invariants to assert continuously
- Store: linearizability; no acknowledged write lost; revisions strictly increase.
- Scheduler: never binds beyond allocatable; never violates required constraints.
- Agent: never two sandboxes for one pod UID; every created resource is in the intent log;
  after GC, no orphans.
- Dataplane: map state equals model of API state after quiescence.
- Security: container fd set, capability set, seccomp mode, NNP, mount visibility.
- Storage: an exclusive volume is never mounted on two nodes simultaneously.
- Controllers: convergence within bound after faults stop; bounded writes after convergence
  (no thrashing).

## Kernel matrix
5.15 (floor), 6.1, 6.6, 6.12 (LTS), latest stable; on x86_64 and aarch64. Kernels from
distro cloud images where possible (realistic configs) plus a minimal custom-config kernel
(to catch missing-feature fallbacks).

## VM test infrastructure (`xtask vm-test`)
- Downloads cloud images (cached, checksum-verified), creates overlay disks, generates
  cloud-init seed (NoCloud) with SSH key, boots with Cloud Hypervisor or QEMU (KVM),
  copies test binaries, runs them as root, collects results/logs.
- Multi-VM topologies with a virtual bridge for cluster tests; `tc netem` / BPF-based fault
  injection on the host bridge.
- Runs locally on any Linux dev box with KVM; CI runners need nested virt or bare metal.
  GitHub-hosted x86_64 Linux runners expose `/dev/kvm` (after a udev permission rule)
  `[verify]`; hosted arm64 runners most likely do not, so aarch64 VM tests need
  self-hosted or bare-metal runners.
- On the macOS dev machine, `xtask vm-test` runs inside the Linux dev VM (`xtask devvm`,
  see [roadmap M0](03-roadmap.md#m0--foundations)), not on macOS itself.

## Fault injection hooks
- `failpoints` (`fail` crate) compiled in test builds at critical points (after intent log
  write, before Raft apply, mid-unpack, during netns wiring) to test crash recovery
  deterministically.

## Regression tests for Kubernetes pain points
Every `done` row in the [pain point tracker](../12-critique/06-pain-point-tracker.md) links
to a test reproducing the Kubernetes failure scenario and showing Rustwain's behaviour
(e.g. PP-62: measure DNS queries per external lookup; PP-65: zero failed requests during
rollout under load).

## CI pipeline
What runs on every push and PR, and how the latency budgets become gates, is specified in
[design 0001 – CI pipeline](../design/0001-ci-pipeline.md).

## Developer loop
- `cargo xtask ci` runs the same fast checks as CI, locally (macOS or Linux).
- `cargo test` for pure crates on any OS.
- `cargo xtask vm-test <suite>` for privileged tests on Linux with KVM.
- `rustwain dev` for manual exploration.

## Build checklist
- [ ] `xtask vm-test` runner with image cache and multi-VM support.
- [ ] Simulation harness crate (`rustwain-testkit`).
- [ ] Failpoints wiring.
- [ ] Fuzz targets for each untrusted-input parser.
- [ ] Nightly matrix workflows.
