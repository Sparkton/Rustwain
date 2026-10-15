# Rustwain Design Library

This directory is the design-first body of work for Rustwain: a ground-up, Rust-native
cluster orchestrator that covers the same problem space as Kubernetes (scheduling and
running workloads across many Linux machines) without copying its implementation.

Nothing here is code yet. Every document is meant to be read, argued with, and then
turned into an implementation plan. Each one ends with **open questions** and a
**build checklist** so the jump from design to work items is mechanical.

## How to read this

Read bottom-up. The system is built from the kernel outward, and every layer above
depends on decisions made below it.

1. **Overview** – what we are building and the vocabulary.
2. **Linux foundations** – the kernel primitives every other layer is assembled from.
3. **Container runtime** and **virtualization** – turning a spec into an isolated process or VM.
4. **Node agent** – the per-machine brain that owns everything on one host.
5. **Networking** and **storage** – the two hardest data planes.
6. **Control plane** – the replicated state store, API, scheduler and controllers.
7. **Workloads** – the user-facing objects built on top.
8. **Security** and **cluster operations** – making it safe and keeping it running.
9. **Efficiency** – budgets and techniques that keep overhead low.
10. **Critique** – honest pros and cons of the existing stack, and a tracker of pain
    points we intend to fix.
11. **Implementation** – crate layout, Rust ecosystem, roadmap, testing.

## Map

### 00 – Overview
| Doc | Covers |
|---|---|
| [01-vision-and-goals](00-overview/01-vision-and-goals.md) | Why Rustwain exists, goals, non-goals, success criteria |
| [02-glossary](00-overview/02-glossary.md) | Rustwain terms mapped to Kubernetes and Linux terms |
| [03-system-architecture](00-overview/03-system-architecture.md) | Every component, how they talk, request walkthroughs |
| [04-design-principles](00-overview/04-design-principles.md) | Rules that settle arguments before they start |
| [05-kubernetes-feature-coverage](00-overview/05-kubernetes-feature-coverage.md) | Every Kubernetes feature mapped to where Rustwain covers it |

### 01 – Linux foundations
| Doc | Covers |
|---|---|
| [01-namespaces](01-linux-foundations/01-namespaces.md) | All eight namespace types, setns/unshare/clone3, pitfalls |
| [02-cgroups-v2](01-linux-foundations/02-cgroups-v2.md) | Unified hierarchy, controllers, PSI, delegation, systemd interplay |
| [03-security-primitives](01-linux-foundations/03-security-primitives.md) | Capabilities, seccomp, LSMs, Landlock, no_new_privs |
| [04-filesystems](01-linux-foundations/04-filesystems.md) | overlayfs, idmapped mounts, mount propagation, pivot_root |
| [05-linux-host-variance](01-linux-foundations/05-linux-host-variance.md) | Distros, kernels, init systems, arches – running on *different* Linux machines |
| [06-process-supervision](01-linux-foundations/06-process-supervision.md) | pidfd, subreapers, signals, zombies, the init problem |

### 02 – Container runtime
| Doc | Covers |
|---|---|
| [01-runtime-architecture](02-container-runtime/01-runtime-architecture.md) | In-process runtime vs CRI, shims, survivability |
| [02-oci-low-level-runtime](02-container-runtime/02-oci-low-level-runtime.md) | The exact create/start sequence, step by step |
| [03-images-and-snapshotters](02-container-runtime/03-images-and-snapshotters.md) | OCI images, registries, content store, lazy pulling |
| [04-rootless-and-userns](02-container-runtime/04-rootless-and-userns.md) | User namespaces for every pod, rootless nodes |

### 03 – Virtualization
| Doc | Covers |
|---|---|
| [01-isolation-spectrum](03-virtualization/01-isolation-spectrum.md) | Process → sandbox → microVM → full VM; how to choose |
| [02-kvm-and-microvms](03-virtualization/02-kvm-and-microvms.md) | KVM, rust-vmm, Firecracker vs Cloud Hypervisor, VM-backed pods |
| [03-userspace-kernel-sandboxes](03-virtualization/03-userspace-kernel-sandboxes.md) | gVisor-style syscall interception |
| [04-full-vm-workloads](03-virtualization/04-full-vm-workloads.md) | Long-lived VMs as first-class workloads, live migration |
| [05-devices-and-passthrough](03-virtualization/05-devices-and-passthrough.md) | VFIO, GPUs, SR-IOV, device plugins, CDI |

### 04 – Node agent
| Doc | Covers |
|---|---|
| [01-node-agent-architecture](04-node-agent/01-node-agent-architecture.md) | Internal structure, state machines, crash recovery |
| [02-pod-lifecycle](04-node-agent/02-pod-lifecycle.md) | Admission → sandbox → init → run → terminate, in detail |
| [03-health-probes](04-node-agent/03-health-probes.md) | Liveness/readiness/startup, cheaper probe mechanics |
| [04-resources-qos-eviction](04-node-agent/04-resources-qos-eviction.md) | Requests/limits, QoS, CPU pinning, NUMA, memory pressure, eviction |
| [05-node-registration-and-heartbeat](04-node-agent/05-node-registration-and-heartbeat.md) | Join, leases, status, partitions |
| [06-logs-exec-attach-port-forward](04-node-agent/06-logs-exec-attach-port-forward.md) | Streaming APIs: logs, exec, attach, port-forward, cp |
| [07-image-and-disk-gc](04-node-agent/07-image-and-disk-gc.md) | Image, container and disk garbage collection |
| [08-sidecars-and-debugging](04-node-agent/08-sidecars-and-debugging.md) | Sidecar ordering, ephemeral debug containers |

### 05 – Networking
| Doc | Covers |
|---|---|
| [01-network-model](05-networking/01-network-model.md) | The contract every pod gets; decisions and why |
| [02-pod-networking](05-networking/02-pod-networking.md) | veth, netkit, bridges, routing, VXLAN/Geneve, BGP |
| [03-ebpf-dataplane](05-networking/03-ebpf-dataplane.md) | Aya-based datapath: TC/XDP/cgroup hooks, maps, fallbacks |
| [04-services-and-load-balancing](05-networking/04-services-and-load-balancing.md) | Virtual IPs, socket-LB, Maglev, session affinity, external LB |
| [05-dns-and-discovery](05-networking/05-dns-and-discovery.md) | Node-local DNS, the ndots problem, discovery API |
| [06-network-policy](05-networking/06-network-policy.md) | Identity-based policy, L3/L4/L7, enforcement points |
| [07-ingress-and-gateway](05-networking/07-ingress-and-gateway.md) | North-south traffic, gateway model |
| [08-ipam-and-dual-stack](05-networking/08-ipam-and-dual-stack.md) | Address allocation, IPv6-first, dual stack |
| [09-encryption-and-multicluster](05-networking/09-encryption-and-multicluster.md) | WireGuard/IPsec, cross-cluster routing |
| [10-service-mesh](05-networking/10-service-mesh.md) | Sidecar-less mTLS, L7 policy, where meshes fit |

### 06 – Storage
| Doc | Covers |
|---|---|
| [01-storage-model](06-storage/01-storage-model.md) | Volume types, claims, lifecycle, topology |
| [02-volume-driver-interface](06-storage/02-volume-driver-interface.md) | A CSI-equivalent plugin contract |
| [03-local-storage](06-storage/03-local-storage.md) | Ephemeral storage, local PVs, LVM/ZFS/btrfs, quotas, snapshots |

### 07 – Control plane
| Doc | Covers |
|---|---|
| [01-api-server](07-control-plane/01-api-server.md) | API surface, transport, versioning, request pipeline |
| [02-state-store](07-control-plane/02-state-store.md) | Embedded Raft + MVCC store replacing etcd |
| [03-object-model](07-control-plane/03-object-model.md) | Metadata, spec/status, field ownership, apply, custom types |
| [04-watch-and-caching](07-control-plane/04-watch-and-caching.md) | Watch protocol, caches, informers, resumption |
| [05-controllers](07-control-plane/05-controllers.md) | Reconciliation runtime, work queues, leader election |
| [06-scheduler](07-control-plane/06-scheduler.md) | Filtering, scoring, preemption, throughput |
| [07-admission-and-policy](07-control-plane/07-admission-and-policy.md) | Validation, mutation, in-process policy instead of webhooks |
| [08-garbage-collection-and-finalizers](07-control-plane/08-garbage-collection-and-finalizers.md) | Owner refs, cascading deletion, finalizers |
| [09-api-fairness](07-control-plane/09-api-fairness.md) | Priority and fairness, rate limiting, overload protection |
| [10-extensibility](07-control-plane/10-extensibility.md) | Custom types, operators, aggregation, controller SDK |

### 08 – Workloads
| Doc | Covers |
|---|---|
| [01-workload-api](08-workloads/01-workload-api.md) | Pod, Service, Deployment/StatefulSet/DaemonSet/Job equivalents |
| [02-rollouts-and-disruption](08-workloads/02-rollouts-and-disruption.md) | Rolling updates, canaries, disruption budgets, drains |
| [03-autoscaling](08-workloads/03-autoscaling.md) | Horizontal, vertical, cluster autoscaling |
| [04-batch-and-gang-scheduling](08-workloads/04-batch-and-gang-scheduling.md) | Queues, gang scheduling, fair share |
| [05-config-and-secret-delivery](08-workloads/05-config-and-secret-delivery.md) | ConfigMaps, downward API, projected volumes, hot reload |
| [06-placement](08-workloads/06-placement.md) | Affinity, taints/tolerations, topology spread |
| [07-quotas-limits-priority](08-workloads/07-quotas-limits-priority.md) | Resource quotas, limit ranges, priority, preemption |

### 09 – Security
| Doc | Covers |
|---|---|
| [01-identity-and-pki](09-security/01-identity-and-pki.md) | Cluster CA, mTLS everywhere, rotation, workload identity |
| [02-authn-authz](09-security/02-authn-authz.md) | Users, service accounts, RBAC and beyond |
| [03-secrets](09-security/03-secrets.md) | Encryption at rest, delivery to nodes, KMS |
| [04-supply-chain](09-security/04-supply-chain.md) | Image signing, provenance, verifying our own releases |
| [05-multi-tenancy](09-security/05-multi-tenancy.md) | Soft vs hard tenancy, what isolation each layer provides |
| [06-workload-security-policy](09-security/06-workload-security-policy.md) | Pod security levels, default hardening |

### 10 – Cluster operations
| Doc | Covers |
|---|---|
| [01-bootstrap-and-join](10-cluster-operations/01-bootstrap-and-join.md) | From zero machines to a cluster |
| [02-upgrades-and-compat](10-cluster-operations/02-upgrades-and-compat.md) | Version skew, rolling upgrades, schema migration |
| [03-ha-and-failure-modes](10-cluster-operations/03-ha-and-failure-modes.md) | What fails, what happens, how we recover |
| [04-heterogeneous-fleets](10-cluster-operations/04-heterogeneous-fleets.md) | Mixed arches, distros, kernels, edge nodes |
| [05-cli-and-ux](10-cluster-operations/05-cli-and-ux.md) | `rwctl`, config format, error messages |
| [06-observability](10-cluster-operations/06-observability.md) | Deferred – design notes only, so we leave the right hooks |
| [07-backup-and-disaster-recovery](10-cluster-operations/07-backup-and-disaster-recovery.md) | Store snapshots, restore, app-level backup |
| [08-packaging-and-gitops](10-cluster-operations/08-packaging-and-gitops.md) | Templating, overlays, apply sets, GitOps |
| [09-cloud-provider-integration](10-cluster-operations/09-cloud-provider-integration.md) | Load balancers, disks, node lifecycle from clouds |
| [10-audit-and-events](10-cluster-operations/10-audit-and-events.md) | Audit trail and events |

### 11 – Efficiency
| Doc | Covers |
|---|---|
| [01-performance-budgets](11-efficiency/01-performance-budgets.md) | Concrete numbers we hold ourselves to |
| [02-density-and-packing](11-efficiency/02-density-and-packing.md) | Bin packing, overcommit, pods per node, idle reclaim |
| [03-startup-latency](11-efficiency/03-startup-latency.md) | Every millisecond between "create" and "running" |

### 12 – Critique of existing software
| Doc | Covers |
|---|---|
| [01-kubernetes-pros-cons](12-critique/01-kubernetes-pros-cons.md) | What Kubernetes gets right and wrong, component by component |
| [02-runtimes-pros-cons](12-critique/02-runtimes-pros-cons.md) | containerd, CRI-O, runc, crun, youki, gVisor, Kata, Firecracker |
| [03-networking-pros-cons](12-critique/03-networking-pros-cons.md) | kube-proxy, Flannel, Calico, Cilium, CoreDNS, service meshes |
| [04-storage-and-etcd-pros-cons](12-critique/04-storage-and-etcd-pros-cons.md) | etcd, CSI, Rook/Ceph, Longhorn, local-path |
| [05-alternatives-landscape](12-critique/05-alternatives-landscape.md) | Nomad, Swarm, k3s, Talos, Borg/Omega papers, Fly.io, ECS |
| [06-pain-point-tracker](12-critique/06-pain-point-tracker.md) | Every known pain point → Rustwain answer → status |

### 13 – Implementation
| Doc | Covers |
|---|---|
| [01-crate-layout](13-implementation/01-crate-layout.md) | Workspace structure and dependency rules |
| [02-rust-ecosystem](13-implementation/02-rust-ecosystem.md) | Which crates we build on, and which we write ourselves |
| [03-roadmap](13-implementation/03-roadmap.md) | Milestones from "run one container" to "multi-node cluster" |
| [04-testing-strategy](13-implementation/04-testing-strategy.md) | Unit, integration, kernel matrix, chaos, fuzzing |

## Conventions used in every doc

- **K8s analogue:** each doc names the Kubernetes component(s) it replaces.
- **Decision records** are written as `Decision:` / `Alternatives:` / `Why:` blocks.
- **Kernel requirements** are stated as a minimum version plus the config flags needed.
- **Status tags**: `[decided]`, `[proposed]`, `[open]` – nothing is final until it has code and tests behind it.
- Each doc ends with **Open questions** and a **Build checklist**.
