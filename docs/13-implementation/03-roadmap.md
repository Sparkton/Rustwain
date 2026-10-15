# Roadmap

Bottom-up, each milestone usable and testable on its own. Durations are deliberately
omitted – this is a learning project; scope is the contract, not dates. Each milestone
lists its **exit criteria**.

```
M0 Foundations ─▶ M1 Single node ─▶ M2 Node networking ─▶ M3 Control plane core
     ─▶ M4 Multi-node cluster ─▶ M5 Storage ─▶ M6 Isolation (microVM)
     ─▶ M7 Production shape ─▶ M8 Advanced (batch, VMs, mesh, autoscaling)
```

## M0 – Foundations
- Workspace skeleton, lints, dependency rules, CI (fmt, clippy, tests, deny/audit).
- `xtask` with VM test runner (boot a cloud image VM with Cloud Hypervisor/QEMU, copy
  binaries, run integration tests as root inside).
- `rustwain-sys`, `rustwain-cgroup` basics, `rustwain-probe` + `rustwain doctor`.
- **Exit**: `rustwain doctor` runs on Ubuntu, Debian, Fedora, Alpine VMs and reports
  correct features.

## M1 – Single-node runtime (`rustwain agent --standalone`)
- Image store: pull (auth, multi-arch), unpack (sandboxed), overlay snapshots, GC basics.
- `rustwain-container` + supervisor + `rustwain-init`.
- Pod spec subset: multiple containers, init containers, sidecars, env, command, volumes
  (scratch, host, config files from local dir), resources (cgroups), restart policy,
  probes (http/tcp/exec), logs, exec/attach (local CLI), userns default, seccomp default.
- Static pods from directory.
- Agent restart without disruption.
- **Exit**: run a multi-container pod (nginx + sidecar) on all Tier-1 distros; kill -9 agent
  mid-run → pods unaffected; security invariant tests pass; pod start p50 < 150 ms warm.

## M2 – Node networking
- Dataplane: netkit/veth setup, local IPAM, eBPF loader with Aya, pinned maps.
- Pod connectivity on one node; host ports; masquerade egress.
- Node-local DNS (static zone from local config at this stage).
- nftables fallback.
- **Exit**: pods on one node reach each other and the internet; `BPF_PROG_TEST_RUN`
  suite green on kernel matrix (5.15, 6.1, 6.6, 6.12, latest).

## M3 – Control plane core
- API types (core kinds), codegen (protobuf/OpenAPI/clients), request pipeline with
  in-memory store backend.
- Store: openraft + log + in-memory MVCC + snapshots + watch; deterministic simulation
  tests.
- AuthN (certs), RBAC basic, admission built-ins, SSA.
- Controller runtime; Deployment, Namespace, GC controllers.
- Scheduler with core plugins.
- `rwctl` basics (get/apply/delete/logs/exec).
- **Exit**: single-server control plane + agent in `rustwain dev` runs a Deployment;
  linearizability checker passes under simulated faults for the store.

## M4 – Multi-node cluster
- 3-server Raft, join flows, PKI with rotation, node leases, node lifecycle.
- Cross-node networking (native routing + Geneve), Services (socket-LB, endpoint
  controller), cluster DNS from watch state, NetworkPolicy (L3/L4 identity-based).
- StatefulSet, DaemonSet, Job/CronJob, disruption budgets, drain.
- Agent dial-out tunnel for logs/exec/port-forward.
- **Exit**: 3 servers + 5 agents in VMs; rolling update of a Deployment behind a Service with
  zero failed requests (drain gate); node kill → pods rescheduled; chaos suite basic.

## M5 – Storage
- Claims/Disks/StorageClasses, local driver (directory+quota, LVM thin), CSI adapter,
  attach controller with fencing, snapshots, expansion.
- Config/Secret delivery with encryption at rest, node bundle stream.
- **Exit**: Postgres StatefulSet with local volumes survives pod restarts; CSI sanity
  suite passes; csi-driver-nfs works through adapter.

## M6 – Isolation: microVMs
- Cloud Hypervisor driver, guest kernel build, `rustwain-guest`, virtio-fs, TAP networking
  with per-packet service path, vsock control proxy.
- Pod overhead accounting; tenant minimum isolation.
- **Exit**: same pod spec runs as `process` and `microvm`; boot < 300 ms p50 cold; network
  policy and services work identically.

## M7 – Production shape
- Upgrades (rolling servers, in-place agents, feature levels), backups/restore, audit,
  events store, API fairness, admission policies (CEL), workload security levels,
  tenants, image signature verification, `rwctl why`, import adapter for Kubernetes YAML.
- Performance budgets enforced in CI; 1,000-node simulated scale test.
- **Exit**: upgrade N−1 → N with live traffic and zero pod restarts; DR drill passes;
  budgets green.

## M8 – Advanced (pick by interest)
- Batch: queues, gang scheduling, JobGroup.
- VirtualMachine kind + live migration.
- Node-level mTLS (mesh L4), gateway (`rustwain-gateway`), Gateway API.
- Autoscaling: Scaler, provisioner with local VM provider, consolidation.
- Devices: DRA-style allocation, CDI, GPUs.
- WireGuard encryption, BGP, bare-metal LB.
- composefs/lazy pulling, P2P images.
- Embedded VMM on rust-vmm (learning milestone).

## Cross-cutting, every milestone
- Update the [pain point tracker](../12-critique/06-pain-point-tracker.md) statuses.
- Keep docs in sync with decisions (`[proposed]` → `[decided]`).
- Add regression tests demonstrating each Kubernetes failure mode we claim to fix.

## How work is executed
Code changes follow the project's agent workflow (orchestrated parallel implementers on
disjoint slices with cross-review), defined in the repo's agent skills. Each milestone is
broken into slices along the crate boundaries in [crate layout](01-crate-layout.md),
which are chosen so slices rarely touch the same files.
