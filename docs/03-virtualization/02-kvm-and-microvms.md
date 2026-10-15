# KVM and MicroVM Pods

**K8s analogue:** Kata Containers (with Cloud Hypervisor/Firecracker/QEMU), Firecracker
containerd, AWS Fargate/Lambda-style isolation.

## KVM in one page

- `/dev/kvm` → `KVM_CREATE_VM` → VM fd → `KVM_CREATE_VCPU` → vCPU fds.
- Guest memory: userspace `mmap`s anonymous (or file/hugepage-backed) memory and
  registers it via `KVM_SET_USER_MEMORY_REGION` – guest physical → host virtual mapping.
- Each vCPU is a host thread looping on `ioctl(KVM_RUN)`. When the guest does something
  needing emulation (MMIO/PIO access to a device, halt, etc.) the ioctl returns with an
  **exit reason**; the VMM handles it and re-enters.
- Interrupts: `irqfd` (eventfd → guest interrupt line) and `ioeventfd` (guest write to an
  address → eventfd signal) let devices run without exiting through the vCPU thread.
- Hardware support: Intel VT-x (`vmx`), AMD-V (`svm`), ARM virtualization extensions
  (EL2). EPT/NPT for second-level address translation.
- Nested: running KVM inside a cloud VM requires the cloud to expose nested virt.

## VMM landscape (Rust)

| VMM | Language | Devices | Boot | Features | Notes |
|---|---|---|---|---|---|
| **Firecracker** | Rust (rust-vmm) | virtio-net, virtio-blk, virtio-vsock, balloon, serial (minimal set, MMIO; PCI added 2025) | Direct kernel boot (no firmware) | Snapshots/restore, rate limiters, jailer | AWS Lambda/Fargate; intentionally minimal; no virtio-fs, no GPU passthrough |
| **Cloud Hypervisor** | Rust (rust-vmm) | virtio-net/blk/fs/pmem/vsock/rng/balloon/mem, vhost-user, VFIO, PCI | Direct kernel boot or UEFI (rust-hypervisor-firmware / OVMF) | Hotplug CPU/mem/devices, live migration, snapshots, TDX/SEV-SNP | Broader; good fit for pods (virtio-fs!) and VMs |
| **crosvm** | Rust | Wide (ChromeOS) | | Sandboxed device processes | Chrome-OS centric |
| **Dragonball** | Rust (Kata built-in VMM) | Kata-focused | | Built into Kata runtime-rs | |
| **libkrun** | Rust (library) | virtio-fs, vsock-based networking (TSI) | Embedded kernel | Library, not process | Used by crun/krun, Podman machine on macOS |
| QEMU | C | Everything | Firmware or direct | Everything | Largest attack surface |

All Rust VMMs share **rust-vmm** crates: `kvm-ioctls`, `kvm-bindings`, `vm-memory`,
`linux-loader`, `vm-superio`, `virtio-queue`, `vhost`, `vhost-user-backend`,
`event-manager`, `vmm-sys-util`, `seccompiler`.

## Decision: how Rustwain runs microVMs `[open]`

| Option | Pros | Cons |
|---|---|---|
| A. Exec Cloud Hypervisor binary, drive via its REST API over a unix socket | Mature, full-featured, no VMM code to maintain | External binary (not "Rust-only from our code" but it *is* Rust); version management |
| B. Exec Firecracker | Smallest attack surface, fastest snapshot restore | No virtio-fs → rootfs must be a block device; no VFIO |
| C. Embed a VMM built on rust-vmm crates into `rustwain-supervisor` | Single binary, tailored device model, deepest learning | Large effort; security responsibility is ours |
| D. Embed libkrun | Library integration | Narrower feature set, TSI networking model differs |

Proposed path: **A first (Cloud Hypervisor), design the `MicroVmBackend` against an
internal `Vmm` trait, then explore C** as a learning milestone once the pod side is
proven. Cloud Hypervisor gives virtio-fs and VFIO which we need for pods and GPUs.

## MicroVM pod anatomy

```
host
 └─ rustwain-supervisor (pod)          cgroup: rustwain.slice/.../pod-<uid>
     ├─ virtiofsd (Rust, vhost-user-fs) ← serves the pod's rootfs + volumes dir
     ├─ VMM process (cloud-hypervisor)  ← cgroup: pod-<uid>/vmm  (vCPU threads, IO threads)
     │    ├─ virtio-net  ← TAP in pod netns (dataplane wires it like a veth)
     │    ├─ virtio-fs   ← vhost-user socket to virtiofsd
     │    ├─ virtio-blk  ← (block volumes, or block rootfs)
     │    ├─ virtio-vsock← supervisor ↔ guest agent
     │    └─ balloon / free page reporting
     └─ guest
         ├─ kernel (shared, read-only file, minimal config, ~10–20 MB)
         └─ rustwain-guest (PID 1 in guest)
              ├─ mounts virtio-fs, sets up guest cgroups
              ├─ configures eth0 from parameters (IP/MTU/routes)
              └─ launches containers with the same `rustwain-container` crate (namespaces
                 inside the guest for container-to-container separation)
```

The guest runs a stripped Linux kernel with: virtio drivers, cgroup v2, namespaces,
overlayfs, vsock, minimal netfilter (none if possible). No modules, no initrd needed if
the guest agent is the init on a virtio-fs/pmem root.

## Boot sequence and latency budget

| Step | Target |
|---|---|
| Supervisor spawns virtiofsd + VMM | 5–10 ms |
| VMM creates VM, loads kernel (direct boot, `linux-loader`) | 5–10 ms |
| Guest kernel boot to init (minimal config, no firmware) | 50–120 ms |
| Guest agent init, mount, net config, vsock connect | 10–20 ms |
| Container start inside guest | 5–20 ms |
| **Total** | **~100–200 ms** |

Faster: **snapshot restore**. Boot one template VM per (kernel, guest-agent version,
vCPU/mem shape), snapshot after the guest agent is ready and idle, then for each new pod
restore from the snapshot (memory file `mmap`ed `MAP_PRIVATE` → CoW), inject per-pod
identity via vsock. Firecracker restores in ~5–30 ms. Caveats:
- **Entropy and uniqueness**: restored VMs share RNG state, TCP sequence seeds, UUIDs.
  Must reseed (`virtio-rng`, VM generation ID / `vmgenid` device to signal clone,
  re-randomize in guest agent before starting workloads).
- Clock jumps: guest must resync time (kvm-clock + guest agent sets time).

## Resources and accounting

- vCPUs: `ceil(cpu limit)` or `ceil(request)` if no limit, min 1. Host-side cgroup on
  the VMM's vCPU threads applies `cpu.weight`/`cpu.max` as for a process pod, so the
  scheduler's math is identical.
- Memory: VM memory = container limits sum + guest overhead (kernel + agent + page cache
  reserve). Memory **hotplug** (virtio-mem) lets us start small and grow on in-place
  resize. Host cgroup on the VMM limits total RSS.
- **Free page reporting** (virtio-balloon feature): the guest reports freed pages, host
  `MADV_DONTNEED`s them → RSS shrinks when the guest frees memory. Essential for density.
- Hugepages for guest memory: better TLB performance, but prevents fine-grained reclaim.
  Opt-in.

## Networking

- Dataplane creates a TAP (instead of netkit/veth peer) inside the pod netns, attaches
  the same BPF programs to the host-side interface. Policy and service LB work unchanged
  because they operate on the host side.
- Socket-level service LB (cgroup/connect hooks) does **not** apply inside the guest
  (guest has its own kernel) → services are resolved by the host-side TC program
  (per-packet DNAT) for VM pods. The dataplane must support both paths.
- vhost-net (in-kernel virtio backend) for throughput; `multiqueue` for multi-vCPU.
- MTU: TAP/virtio MTU must match the pod network MTU (overlay overhead).

## Storage

- Rootfs: **virtio-fs** sharing the host overlay mount (DAX optional for page-cache
  sharing between host and guest – reduces double caching). Or a **block** rootfs (ext4
  image from the snapshotter) via virtio-blk – faster metadata ops, no host page cache
  sharing, needs a block snapshotter.
- Volumes: filesystem volumes via virtio-fs; block volumes via virtio-blk with
  `io_uring` backend in the VMM.
- `virtiofsd` (Rust, from the virtio-fs project) is the reference daemon. It must run
  sandboxed (it does in its own namespaces by default).

## Guest agent (`rustwain-guest`)

- Static Rust binary, PID 1 in the guest.
- vsock server with the **same control protocol** as the supervisor (start container,
  exec, signal, stats, logs). The supervisor proxies – the agent doesn't care whether a
  pod is a VM.
- Responsibilities: mount virtio-fs shares, set hostname, configure network, set up guest
  cgroups, run containers using `rustwain-container`, reap, stream logs back over vsock,
  respond to probes (exec probes run inside the guest; HTTP/TCP probes can run from the
  host side against the pod IP).
- Minimal attack surface: the vsock port only accepts the supervisor (vsock CID 2 = host).

## Security considerations

- VMM process runs: in its own user (or userns), cgroup, minimal seccomp
  (Firecracker's `jailer` model: chroot + seccomp + cgroup + namespaces per VM).
- Host kernel attack surface from a VM: KVM ioctl handling, virtio backends (vhost-net in
  kernel; virtiofsd in userspace). Keep vhost-user backends in separate sandboxed
  processes.
- **Confidential computing**: AMD SEV-SNP and Intel TDX encrypt guest memory against the
  host. Cloud Hypervisor supports TDX (and SEV-SNP work in progress/available on some
  forks). Rustwain could expose `isolation: microvm, confidential: true` → schedule to
  capable nodes, attestation before secrets delivery. `[later]`.

## Open questions

- Cloud Hypervisor exec vs embedded VMM (see decision above).
- Snapshot-restore templates: how many shapes to pre-warm, memory cost of templates.
- Do containers inside the guest still need namespaces? Yes if a pod has multiple
  containers with different images; cheap inside the guest.

## Build checklist

- [ ] `Vmm` trait; Cloud Hypervisor implementation (REST over unix socket).
- [ ] Guest kernel config + build reproducibly (`cargo xtask guest-kernel`).
- [ ] `rustwain-guest` crate (shares `rustwain-container`).
- [ ] vsock control protocol proxy in supervisor.
- [ ] Dataplane TAP support + per-packet service DNAT path.
- [ ] Measure boot latency and per-VM overhead; template snapshot spike.
