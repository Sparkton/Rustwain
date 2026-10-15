# The Isolation Spectrum

**K8s analogue:** RuntimeClass + Kata Containers / gVisor / KubeVirt.

Kubernetes treats "how is this pod isolated?" as an external plugin choice
(RuntimeClass → a different CRI handler). Rustwain treats it as a first-class property
of the workload: the **isolation class**.

## The classes

| Class | Boundary | Kernel shared with host? | Startup | Memory overhead | Syscall overhead | Typical use |
|---|---|---|---|---|---|---|
| `process` | Namespaces + cgroups + seccomp + LSM + userns | Yes | ~10–50 ms | ~0 | 0 | Trusted/first-party workloads, most services |
| `sandboxed` | process + strict seccomp + Landlock + (optional) userspace kernel | Partially (narrow syscall surface) | ~50–150 ms | 10–50 MB (userspace kernel) | moderate–high (userspace kernel) | Semi-trusted code, plugins, CI jobs |
| `microvm` | KVM hardware virtualization, minimal VMM, guest kernel | No | ~100–300 ms | ~20–40 MB per VM + guest page cache | near-native CPU; IO via virtio | Multi-tenant, untrusted code, serverless |
| `vm` | Full VM (UEFI, arbitrary OS, disk images) | No | seconds | guest-defined | near-native | Legacy apps, Windows guests, appliances, kernels with special modules |

Figures are typical published numbers (Firecracker: ~125 ms to user space, <5 MiB VMM
overhead; Cloud Hypervisor similar order) and will be replaced by our own measurements.

## What each class defends against

| Threat | process | sandboxed | microvm | vm |
|---|---|---|---|---|
| Accidental resource hogging | ✔ cgroups | ✔ | ✔ | ✔ |
| Reading other pods' files | ✔ mount ns + userns + LSM | ✔ | ✔ | ✔ |
| Kernel exploit via syscall bug | ✘ (seccomp reduces surface) | partial (much smaller surface) | ✔ (guest kernel exploited only; host reachable only via VMM/KVM bugs) | ✔ |
| Container runtime bug (escape at setup) | ✘ mitigated by userns | partial | ✔ | ✔ |
| Side channels (Spectre-class, cache timing) | ✘ | ✘ | partial (needs core scheduling / dedicated cores) | partial |
| Malicious device access | ✔ device cgroup | ✔ | ✔ (only virtio devices) | ✔ |

**Core scheduling** (`prctl(PR_SCHED_CORE)`, 5.14+) prevents processes from different
trust domains sharing SMT siblings simultaneously – mitigates L1TF/MDS-class leaks. The
agent can tag each pod (or each tenant) as a core-scheduling cookie group for
`microvm`/`sandboxed` classes. Costs throughput on SMT machines; opt-in per tenant.

## Same pod spec, different class

```yaml
kind: Pod
spec:
  isolation: microvm          # process | sandboxed | microvm | vm(only for VirtualMachine kind)
  containers:
    - name: app
      image: ghcr.io/acme/app@sha256:...
      resources: { requests: { cpu: 500m, memory: 256Mi }, limits: { memory: 512Mi } }
```

The agent picks the backend for the class. Differences that leak through and must be
handled:

| Aspect | process | microvm |
|---|---|---|
| Memory limit | cgroup on container | VM memory size = pod limit + guest overhead; containers inside also get cgroups in the guest |
| CPU | cgroup weight/quota | vCPU count = ceil(limit or request); host cgroup on VMM threads |
| Volumes | bind mounts | virtio-fs (shared dir) or virtio-blk (block volumes) |
| Networking | netns + netkit/veth | TAP device in pod netns → virtio-net in guest; or vhost-net/vhost-user |
| Exec/attach/logs | supervisor pipes | vsock streams to guest agent |
| Image rootfs | overlay on host | virtio-fs share of overlay dir, or block device from snapshot |
| Pod overhead | ~0 | Added automatically to requests (Kubernetes `RuntimeClass.overhead`) |
| GPUs | device nodes + cgroup | VFIO passthrough to VM |
| Probes (exec type) | in-ns exec | via guest agent |

## Choosing a class

Policy layers:
1. **Tenant default**: e.g. external customers' tenant → `microvm` default and minimum.
2. **Workload request**: may raise isolation, never lower below tenant minimum.
3. **Node capability**: scheduler only places `microvm` pods on nodes with KVM.

Decision `[proposed]`: classes are an ordered lattice
`process < sandboxed < microvm`; `vm` is a separate workload kind
(`VirtualMachine`) because it doesn't run OCI containers.

## Overhead accounting

Pod overhead per class is computed by the agent and **reported to the scheduler as part
of node capacity math**:
- `microvm`: VMM RSS (~5–15 MB) + guest kernel + guest agent + guest page cache
  reservation (configurable) ≈ 30–60 MB default; CPU overhead ~ small fraction for IO
  threads.
- Without accounting, nodes overcommit silently (a Kata pain point in Kubernetes before
  PodOverhead existed).

## Interaction with density

MicroVMs cost memory per pod. Techniques to reduce (see
[KVM and microVMs](02-kvm-and-microvms.md)):
- **Memory ballooning / free page reporting** to return unused guest memory.
- **Shared, read-only guest kernel and initrd** pages (DAX-mapped kernel, virtio-pmem).
- **KSM** (kernel same-page merging) across VMs – reduces memory but has side-channel
  implications (cross-VM dedup timing attacks) → off for multi-tenant, on for trusted.
- **Snapshots/restore** from a pre-booted template (Firecracker snapshots) – both faster
  start and shared clean pages via `MAP_PRIVATE` of the memory file.

## Build order

1. `process` (milestone 1).
2. `microvm` (milestone 3) – biggest isolation win per effort, and Rust has the best VMM
   ecosystem (rust-vmm).
3. `vm` kind (milestone 4+).
4. `sandboxed` – least clear; may end up being "process + aggressive hardening" rather
   than a userspace kernel. See [userspace kernel sandboxes](03-userspace-kernel-sandboxes.md).

## Open questions

- Should `sandboxed` exist at all, or should it be removed in favour of a well-hardened
  `process` + `microvm`?
- Is per-tenant core scheduling worth the throughput hit as a default for `microvm`?

## Build checklist

- [ ] `IsolationClass` enum, lattice ordering, tenant min/default policy.
- [ ] Per-class overhead calculator feeding scheduler capacity.
- [ ] Node attribute: supported classes.
- [ ] Core scheduling cookie management per tenant (opt-in).
