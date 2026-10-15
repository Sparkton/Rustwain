# Userspace-Kernel Sandboxes

**K8s analogue:** gVisor (`runsc`) via RuntimeClass.

## The idea

Instead of letting the workload's syscalls reach the host kernel, intercept them and
service them in a **userspace kernel** that reimplements Linux semantics. The host kernel
only sees the small set of syscalls the sandbox kernel itself makes (gVisor's Sentry uses
~50–70 host syscalls, filtered by seccomp).

## How gVisor does it

- **Sentry**: a Go program implementing much of the Linux syscall ABI (process model,
  VFS, networking stack "netstack", signals, mm).
- **Gofer** (now partly replaced by "directfs"): a separate process mediating file access.
- **Platforms** (how syscalls are intercepted):
  - `ptrace` – slow (two context switches per syscall), works everywhere.
  - `KVM` – runs the Sentry as a guest kernel using KVM for address-space isolation;
    fast on bare metal, poor in nested virt.
  - `systrap` (default since 2023) – seccomp `SECCOMP_RET_TRAP` + signal handler in a stub,
    shared-memory handoff to Sentry; good performance without KVM.
- Strengths: very small host attack surface, no VM memory overhead, works where KVM
  isn't available.
- Weaknesses: syscall-heavy workloads pay 2–10×+ overhead on syscalls; IO and networking
  through netstack are slower than host; compatibility gaps (some syscalls, `/proc`
  details, io_uring disabled, some ioctls); large codebase to trust.

## Options for Rustwain's `sandboxed` class

| Option | Effort | Isolation | Performance |
|---|---|---|---|
| **1. Integrate gVisor (`runsc`)** via OCI/CRI-ish adapter | Low–medium | High | gVisor's |
| **2. Hardened process** – `process` + tight per-workload seccomp (recorded), Landlock, no nested userns, read-only rootfs, core scheduling, BPF-LSM rules | Low | Medium (host kernel still exposed but surface narrowed) | Native |
| **3. Write a Rust userspace kernel** | Enormous (gVisor is ~hundreds of kLOC) | High | Unknown |
| **4. microVM with tiny footprint** (libkrun-style or Firecracker snapshot restore) | Medium (shared with microvm work) | High | Near-native CPU, virtio IO |

Decision `[proposed]`:
- Define `sandboxed` as **option 2** (a hardened process profile) – cheap, useful, no
  new trust base.
- Provide **option 1** as an optional backend for users who want gVisor specifically.
- For "real" kernel isolation, point users at `microvm` (option 4) – with snapshot
  restore it's often comparable in startup and lower in syscall overhead.
- Option 3 is explicitly **not** planned. It's a fascinating learning project, but it
  is a separate product.

## Hardened process profile (option 2) contents

- Seccomp: allowlist generated from recording (`rwctl seccomp record`), falling back to a
  "strict default" much tighter than RuntimeDefault (no `ptrace`, no `perf_event_open`,
  no `bpf`, no `io_uring_setup` (io_uring has been a major source of kernel CVEs –
  Google disabled it on ChromeOS/Android/production servers), no `userfaultfd`, no
  `keyctl`, no namespace creation, no `mount`).
- Landlock: filesystem writes restricted to volumes + `/tmp`; network restricted to
  declared ports (ABI v4+).
- Read-only rootfs enforced.
- No capabilities at all; runs as non-root UID inside userns.
- `/proc` mounted with `subset=pid,hidepid=invisible`.
- Core scheduling cookie per pod.
- BPF-LSM hooks (where available) denying `ptrace` and `kernel_module` operations.
- Optionally `prctl(PR_SET_MDWE)` (6.3+, memory-deny-write-execute) for workloads without
  JIT.

## Open questions

- Is "sandboxed = hardened process" misleading naming? It is *not* a kernel boundary.
  Possibly rename to `process-strict` and reserve `sandboxed` for gVisor integration.
- How to keep recorded seccomp profiles maintained as apps change (re-record in CI?).

## Build checklist

- [ ] Strict seccomp profile + Landlock generator from volume list.
- [ ] Optional gVisor backend spike (run `runsc` with a generated OCI bundle).
- [ ] Benchmark syscall-heavy workload (e.g. `redis-benchmark`, file-heavy builds) across
      process / process-strict / gVisor / microvm.
