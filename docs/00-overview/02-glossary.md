# Glossary

Rustwain terms, the closest Kubernetes analogue, and the Linux/virtualization concept
underneath. Where we reuse the Kubernetes word, it is because the concept is the same
and inventing a new word would only add friction.

## Cluster-level

| Rustwain | Kubernetes | Meaning |
|---|---|---|
| **Cluster** | Cluster | A set of servers and agents sharing one state store. |
| **Server** | Control-plane node | A machine running `rustwain server`: API, store replica, scheduler, controllers. |
| **Agent** | Worker node / kubelet | A machine running `rustwain agent`. A server can also be an agent. |
| **Store** | etcd | The embedded, Raft-replicated MVCC key-value store inside each server. |
| **API** | kube-apiserver | The request front door: authn, authz, admission, validation, persistence, watch. |
| **Scheduler** | kube-scheduler | Assigns pods to nodes. |
| **Controller** | controller in kube-controller-manager | A reconciliation loop that drives actual state toward desired state. |
| **Namespace (API)** | Namespace | A naming and policy scope for objects. Not the same as a Linux namespace – docs always say "API namespace" or "Linux namespace" when ambiguous. |
| **Tenant** | (none; usually namespace + policy) | A first-class boundary for quota, identity and isolation defaults. |

## Workload-level

| Rustwain | Kubernetes | Meaning |
|---|---|---|
| **Pod** | Pod | The scheduling unit: one or more containers sharing a network identity, volumes and a lifecycle, placed on one node. |
| **Container** | Container | One process tree with its own root filesystem image, inside a pod. |
| **Sandbox** | Pod sandbox / pause container | The isolation envelope for a pod: Linux namespaces + cgroup, or a VM. Rustwain has **no pause container**; the sandbox is held by the supervisor. |
| **Isolation class** | RuntimeClass | `process`, `sandboxed`, `microvm`, `vm`. Chooses how the sandbox is built. |
| **Supervisor** | containerd-shim | The tiny per-pod process that holds namespaces, reaps children and outlives agent restarts. |
| **Service** | Service | A stable virtual address and DNS name for a set of pods. |
| **Endpoint set** | EndpointSlice | The resolved list of pod addresses behind a Service. |
| **Deployment / StatefulSet / DaemonSet / Job** | same | Workload controllers. We keep the names because they are well understood. |
| **Rollout** | (part of Deployment) | A first-class object describing a change in progress. |
| **Volume** | Volume | Storage attached to a pod. |
| **Claim** | PersistentVolumeClaim | A request for persistent storage. |
| **Disk** | PersistentVolume | A provisioned piece of persistent storage. |
| **Storage class** | StorageClass | A template for provisioning disks. |
| **Probe** | Probe | Liveness, readiness or startup check. |

## Node-level

| Rustwain | Kubernetes / Linux | Meaning |
|---|---|---|
| **Node agent** | kubelet + kube-proxy + container runtime + CNI daemon | The single process per machine that owns pods, networking dataplane, volumes and images. |
| **Dataplane** | kube-proxy + CNI plugin | The eBPF programs and maps that move packets. |
| **Image store** | containerd content store + snapshotter | Content-addressed blobs and unpacked layers. |
| **Node feature set** | Node Feature Discovery labels | Detected kernel, CPU, device and distro capabilities, published as node attributes. |
| **Lease** | Lease (node heartbeat) | A short-lived record proving a node is alive. |

## Linux terms used throughout

| Term | Meaning |
|---|---|
| **Linux namespace** | Kernel feature that gives a process its own view of a global resource (PIDs, mounts, network stack, users…). See [namespaces](../01-linux-foundations/01-namespaces.md). |
| **cgroup** | Control group: the kernel's resource accounting and limiting tree. Rustwain requires cgroup v2. See [cgroups v2](../01-linux-foundations/02-cgroups-v2.md). |
| **Capability** | A slice of root's privileges (e.g. `CAP_NET_ADMIN`). |
| **seccomp** | System-call filtering with a BPF program. |
| **LSM** | Linux Security Module: SELinux, AppArmor, Landlock, BPF-LSM. |
| **overlayfs** | Union filesystem used to stack image layers. |
| **idmapped mount** | A mount that translates UIDs/GIDs, letting user-namespaced containers use shared image layers without chown. |
| **pidfd** | A file descriptor referring to a process; race-free signalling and waiting. |
| **eBPF** | Verified programs that run in the kernel at hook points (network, cgroup, tracing, LSM). |
| **BTF / CO-RE** | BPF Type Format / Compile Once – Run Everywhere: lets one eBPF binary adapt to different kernel struct layouts. |
| **veth / netkit** | Virtual ethernet pairs; netkit is a newer (6.7+) BPF-programmable device pair with lower overhead. |
| **VXLAN / Geneve** | UDP-encapsulated overlay networks. |
| **conntrack** | The kernel's connection-tracking table used by NAT and stateful firewalling. |
| **KVM** | The kernel's hardware virtualization interface (`/dev/kvm`). |
| **VMM** | Virtual machine monitor – the userspace program driving KVM (Firecracker, Cloud Hypervisor, QEMU). |
| **virtio** | Paravirtualized device standard for VMs (net, blk, fs, vsock). |
| **vsock** | Host↔guest socket transport with no networking setup. |
| **VFIO** | Kernel framework for safely giving devices directly to userspace/VMs. |
| **PSI** | Pressure Stall Information: how long tasks stalled waiting on CPU, memory or IO. |

## Status tags

- `[decided]` – we have committed to this; changing it needs a new decision record.
- `[proposed]` – the current best idea, open to change.
- `[open]` – genuinely undecided; options are listed.
