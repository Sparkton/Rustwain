# Rustwain
Rusted Kubernetes

Rustwain is a ground-up, Rust-native cluster orchestrator that covers the Kubernetes problem
space: containers, microVMs and VMs, networking, storage, scheduling and operations across
many kinds of Linux machine. It aims for low overhead and failure behaviour you can
understand. It is not a Kubernetes clone. It keeps the useful ideas (declarative API,
reconciling controllers, pods, services) and redesigns the parts that cause the most pain.

> **Status: design phase.** No code yet. The [design library](docs/README.md) (89 docs) is
> the source of truth for what gets built, and [implementation starts at milestone M0](docs/13-implementation/03-roadmap.md).

## Why

Kubernetes works, but it is expensive to run and hard to reason about. Etcd scaling, CRI and
shim layering, kube-proxy and iptables churn, DNS `ndots` amplification, opaque scheduling
failures, and node agents that use hundreds of MiB all add up. Rustwain is a learning project
that rebuilds the stack end to end. It has two aims: to understand every layer, and to fix
(or at least make visible) the problems tracked in the
[pain point tracker](docs/12-critique/06-pain-point-tracker.md).

## Goals and non-goals

| Goals | Non-goals |
|---|---|
| Cover the same functional surface as Kubernetes ([coverage matrix](docs/00-overview/05-kubernetes-feature-coverage.md)) | Wire compatibility with the Kubernetes API (an import adapter for YAML is planned instead) |
| Node agent under 25 MB RSS idle and a supervisor under 1 MB per pod ([budgets](docs/11-efficiency/01-performance-budgets.md)) | Supporting cgroup v1 or kernels older than 5.15 |
| Secure by default: user namespaces, seccomp, and microVM isolation as a first-class option | Being a general-purpose container engine for laptops |
| Run on many distros and architectures ([host variance](docs/01-linux-foundations/05-linux-host-variance.md), [kernel matrix](docs/01-linux-foundations/07-kernel-feature-matrix.md)) | Matching Kubernetes release by release |
| Failures that explain themselves (`rwctl why`, structured conditions) | |

Full detail: [vision and goals](docs/00-overview/01-vision-and-goals.md) and
[design principles](docs/00-overview/04-design-principles.md).

## Architecture at a glance

```
                ┌──────────────────── rustwain server (×3) ────────────────────┐
  rwctl ──gRPC──▶ API server ─ authn/z ─ CEL admission ─ fairness             │
                │      │                                                      │
                │  store: embedded Raft (openraft) + in-memory MVCC = watch   │
                │      │                                                      │
                │  scheduler · controllers · PKI                              │
                └──────┬───────────────────────────────────────────────────────┘
                       │ watch / status (agent dials out)
        ┌──────────────▼──────────── rustwain agent (every node) ─────────────┐
        │ pod manager · probes · eviction · image store · volumes · devices   │
        │   ├─ per-pod supervisor ─▶ containers (namespaces + cgroup v2)      │
        │   └─ microVM backend ─▶ Cloud Hypervisor ─▶ rustwain-guest          │
        │ dataplane: eBPF (Aya) socket-LB + policy, nftables fallback         │
        │ node-local DNS · WireGuard · node-level mTLS                        │
        └─────────────────────────────────────────────────────────────────────┘
```

The main design decisions:

| Area | Decision | Replaces in Kubernetes |
|---|---|---|
| State | Embedded Raft with an in-memory MVCC store that also serves as the watch cache | etcd plus the apiserver watch cache |
| Runtime | In-process runtime plus a tiny per-pod supervisor | CRI, containerd, shims, pause container |
| Isolation | Namespaces with userns and seccomp on by default; Cloud Hypervisor microVMs as a per-pod choice | runc defaults, RuntimeClass plus Kata |
| Networking | eBPF dataplane: socket-level LB, identity-based policy, netkit | kube-proxy iptables/IPVS plus a CNI |
| DNS | Node-local resolver with autopath, fed by watch state | CoreDNS plus `ndots:5` search expansion |
| API | gRPC (tonic), server-side apply, CEL admission | REST plus webhooks |
| Host | cgroup v2 only, kernel 5.15 or newer, single static binary | Wide legacy support |

More: [system architecture](docs/00-overview/03-system-architecture.md) and
[glossary](docs/00-overview/02-glossary.md).

## Design library

Start at the [docs index](docs/README.md) for the reading order. Sections:

| # | Section | What it covers |
|---|---|---|
| 00 | [Overview](docs/00-overview/) | Vision, glossary, architecture, principles, Kubernetes feature coverage |
| 01 | [Linux foundations](docs/01-linux-foundations/) | Namespaces, cgroup v2, seccomp/LSM/Landlock, filesystems, distro variance, supervision, kernel feature matrix |
| 02 | [Container runtime](docs/02-container-runtime/) | Runtime architecture, OCI low-level runtime, images and snapshotters, rootless and userns |
| 03 | [Virtualization](docs/03-virtualization/) | Isolation spectrum, KVM and microVMs, gVisor-style sandboxes, full VMs, device passthrough |
| 04 | [Node agent](docs/04-node-agent/) | Agent architecture, pod lifecycle, probes, QoS and eviction, heartbeats, logs/exec, GC, sidecars |
| 05 | [Networking](docs/05-networking/) | Network model, pod networking, eBPF dataplane, services, DNS, policy, gateway, IPAM, encryption, mesh |
| 06 | [Storage](docs/06-storage/) | Storage model, volume driver interface (CSI adapter), local storage |
| 07 | [Control plane](docs/07-control-plane/) | API server, state store, object model, watch, controllers, scheduler, admission, GC, fairness, extensibility |
| 08 | [Workloads](docs/08-workloads/) | Workload API, rollouts, autoscaling, batch and gang scheduling, config and secrets, placement, quotas |
| 09 | [Security](docs/09-security/) | Identity and PKI, authn/authz, secrets, supply chain, multi-tenancy, workload security policy |
| 10 | [Cluster operations](docs/10-cluster-operations/) | Bootstrap, upgrades, HA and failure modes, heterogeneous fleets, CLI, observability, backup, GitOps, cloud, audit |
| 11 | [Efficiency](docs/11-efficiency/) | Performance budgets, density and packing, startup latency |
| 12 | [Critique](docs/12-critique/) | Pros and cons of Kubernetes, runtimes, networking, storage and etcd; alternatives; pain point tracker |
| 13 | [Implementation](docs/13-implementation/) | Crate layout, Rust ecosystem choices, roadmap, testing strategy |

Every component doc ends with a **build checklist**. Decisions are tagged `[decided]`,
`[proposed]` or `[open]`.

## Roadmap

```
M0 Foundations ─▶ M1 Single node ─▶ M2 Node networking ─▶ M3 Control plane core
     ─▶ M4 Multi-node ─▶ M5 Storage ─▶ M6 microVMs ─▶ M7 Production shape ─▶ M8 Advanced
```

Each milestone has exit criteria that can be tested, for example "kill -9 the agent and pods
are unaffected" or "rolling update behind a Service with zero failed requests". See the
[roadmap](docs/13-implementation/03-roadmap.md),
[crate layout](docs/13-implementation/01-crate-layout.md) and
[testing strategy](docs/13-implementation/04-testing-strategy.md).

## Supported hosts (planned)

- Linux 5.15 or newer (or a vendor kernel with equivalent backports, such as RHEL 9) with
  cgroup v2 (unified hierarchy) on x86_64 and aarch64. Features are probed, never inferred
  from the version.
- Tier 1: Ubuntu LTS, Debian, Fedora, RHEL/Rocky/Alma. Tier 2: Amazon Linux 2023,
  Flatcar/Fedora CoreOS, Alpine (musl). Tier 3: Arch, NixOS, openSUSE.
- KVM is optional and only needed for microVM and VM workloads.
- `rustwain doctor` reports which features each host supports and the fallbacks it will use.

## Contributing

The project is design-first. A change to behaviour starts as an update to the relevant doc
(and the pain point tracker, if it applies) before any code. Agent and contributor workflow,
branch naming (`sparkton/<brief>`) and PR conventions are in [AGENTS.md](AGENTS.md).
