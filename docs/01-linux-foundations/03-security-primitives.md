# Linux Security Primitives

**K8s analogue:** `securityContext` (capabilities, seccompProfile, appArmorProfile,
seLinuxOptions, privileged, allowPrivilegeEscalation, readOnlyRootFilesystem),
Pod Security Standards, and what runc applies.

Namespaces control what a process *sees*; cgroups control what it *uses*; this layer
controls what it is *allowed to do*. Defence in depth: each layer assumes the others
might fail.

## Layering at a glance

| Layer | Mechanism | Granularity | Default in Rustwain |
|---|---|---|---|
| Identity | User namespace + non-root UID | Process | userns on, UID mapped |
| Privilege | Capabilities | ~41 privileges | Minimal set (below) |
| Escalation | `no_new_privs` | Process tree | On |
| Syscalls | seccomp-BPF | Per syscall + args | Default profile on |
| Objects | LSM (SELinux / AppArmor) | Files, sockets, labels | Distro-dependent profile |
| Filesystem sandbox | Landlock | Path hierarchies | `[proposed]` for supervisor itself |
| Resource visibility | Masked/readonly `/proc`, `/sys` paths | Paths | On |
| Kernel attack surface | Sandbox (gVisor-like) or VM | Whole kernel | Optional isolation class |

## Capabilities

Root's power is split into capabilities (`man 7 capabilities`). Each thread has five
sets:

| Set | Meaning |
|---|---|
| Permitted | Upper bound of what can be made effective |
| Effective | What the kernel checks right now |
| Inheritable | Preserved across `execve` (only with file inheritable caps) |
| Bounding | Upper bound for anything gained via exec; can only be reduced |
| Ambient (4.3+) | Preserved across `execve` of non-setuid programs – the useful way to give a non-root process a capability |

### Default set

Docker/containerd default set (14): `CHOWN, DAC_OVERRIDE, FSETID, FOWNER, MKNOD, NET_RAW,
SETGID, SETUID, SETFCAP, SETPCAP, NET_BIND_SERVICE, SYS_CHROOT, KILL, AUDIT_WRITE`.

Rustwain default `[proposed]` (smaller): `CHOWN, DAC_OVERRIDE, FOWNER, FSETID, KILL,
SETGID, SETUID, SETPCAP, NET_BIND_SERVICE`.

Removed and why:
- `NET_RAW` – enables ICMP/ARP spoofing within the pod network. Ping still works via
  `net.ipv4.ping_group_range` sysctl (set per pod netns to cover the container's GIDs).
- `MKNOD` – device nodes should come from the device list, not be created ad hoc.
- `AUDIT_WRITE`, `SYS_CHROOT`, `SETFCAP` – rarely needed; add per workload.

With **user namespaces on by default**, these capabilities apply only within the pod's
userns – `CAP_DAC_OVERRIDE` cannot override permissions on host-owned files. This makes
even the default set far less dangerous than in Kubernetes' default (no userns).

### Applying capabilities correctly

Order in the container init path (pre-exec):

1. `prctl(PR_SET_KEEPCAPS, 1)` if changing UID while keeping caps.
2. `setresgid` / `setgroups` / `setresuid` to the target IDs.
3. Drop bounding set (`PR_CAPBSET_DROP` for each cap not allowed).
4. `capset` for permitted/effective/inheritable.
5. Raise ambient caps (`PR_CAP_AMBIENT_RAISE`) if the target user is non-root and caps
   are requested.
6. `prctl(PR_SET_NO_NEW_PRIVS, 1)`.
7. Load seccomp (must be after NNP if unprivileged).
8. `execve`.

Crate: `caps` (pure Rust) or a thin wrapper over `capset`/`prctl` via `rustix`.

## no_new_privs

`PR_SET_NO_NEW_PRIVS` makes `execve` unable to grant privileges: setuid bits and file
capabilities are ignored. Equivalent to Kubernetes `allowPrivilegeEscalation: false`.
Rustwain sets it **by default**; turning it off requires the `privileged` security level.
Side benefit: seccomp filters can be installed without `CAP_SYS_ADMIN`.

## seccomp

seccomp-BPF attaches a classic-BPF program evaluated on every syscall, seeing the syscall
number, architecture and the six raw arguments (not pointed-to memory).

Actions: `SCMP_ACT_ALLOW`, `KILL_PROCESS`, `KILL_THREAD`, `ERRNO(n)`, `TRAP`, `TRACE`,
`LOG`, `USER_NOTIF` (5.0+).

### Default profile

Docker's default profile allows ~300 syscalls and blocks ~44 dangerous ones (e.g.
`kexec_load`, `open_by_handle_at`, `init_module`, `bpf` without caps, `keyctl`,
`add_key`, `userfaultfd` (unprivileged), `ptrace` on old kernels, `perf_event_open`,
`mount` (unless CAP_SYS_ADMIN), `unshare`/`clone` with namespace flags unless
CAP_SYS_ADMIN, `personality` with unusual values).

Kubernetes runs pods **Unconfined** by default unless `seccompProfile: RuntimeDefault`
is set (or the kubelet `SeccompDefault` feature is enabled, default on since 1.27 only if
configured). Rustwain: **RuntimeDefault always on**; `Unconfined` requires the
privileged level.

### Generating BPF in Rust

- `seccompiler` (from Firecracker, pure Rust) compiles a JSON/Rust-defined filter to BPF
  – no libseccomp C dependency. Supports argument matching.
- `libseccomp-rs` binds the C library (more complete, C dependency).

Decision `[proposed]`: `seccompiler` – pure Rust, battle-tested in Firecracker, and we
precompile the default profile at build time per architecture (x86_64, aarch64 – syscall
numbers differ, and x86_64 must also block the x32 ABI and handle `AUDIT_ARCH` checks so
32-bit syscalls via `int 0x80` cannot bypass the filter).

### seccomp user notification

`SECCOMP_RET_USER_NOTIF` pauses the syscall and sends a notification over an fd to a
supervisor process, which can inspect the target's memory and either let the syscall
continue, fail it, or perform it *on behalf of* the process (with `SECCOMP_ADDFD` to
inject fds). Uses:

- Allow `mount` of specific filesystem types (e.g. sysfs, procfs, specific FUSE) inside
  unprivileged containers – the supervisor validates and performs it.
- Emulate `mknod` for a whitelist of device nodes.
- Syscall auditing/learning mode: record which syscalls a workload uses to build a
  tight custom profile ("profile recorder" → `rwctl seccomp record`).

Cost: a context switch per intercepted syscall – only use it for rare syscalls.

## LSMs: SELinux and AppArmor

Distros pick one major LSM (plus Landlock/Yama/BPF stacked):

| Distro family | Major LSM |
|---|---|
| RHEL, Fedora, CentOS Stream, Rocky, Alma, Amazon Linux 2023 | SELinux (enforcing) |
| Ubuntu, Debian, SUSE (pre-SLE16 / Tumbleweed switched to SELinux in 2025) | AppArmor |
| Arch, Alpine, Gentoo | none by default (configurable) |
| Flatcar, Bottlerocket | SELinux (Bottlerocket), none/SELinux (Flatcar) |

### SELinux

- Every file/process has a label `user:role:type:level`. Containers run as
  `container_t` (from `container-selinux` policy) with a unique **MCS** category pair
  (`s0:c123,c456`) per pod, so two pods can't touch each other's files even if both
  escape to the same type.
- Volumes must be relabelled to the pod's context (`:z` / `:Z` in Docker). Recursive
  relabel of large volumes is slow – Kubernetes added `SELinuxMountReadWriteOncePod` to
  use the `context=` mount option instead of relabelling every file. Rustwain uses
  **mount-time `context=`** whenever the volume is exclusive to one pod.
- Rustwain needs the `container-selinux` policy (or our own policy module) installed on
  SELinux hosts. Preflight checks for the `container_t` type.
- Apply via `setexeccon` before exec (write `/proc/thread-self/attr/exec`) and label
  mounts with `context=`.

### AppArmor

- Path-based profiles loaded into the kernel (`apparmor_parser`), applied on exec via
  `/proc/thread-self/attr/apparmor/exec` (or legacy `attr/exec`) with `exec <profile>`.
- Docker/containerd ship a `docker-default`/`cri-containerd.apparmor.d` profile that
  denies writes to `/proc/sys`, mounts, etc. Rustwain ships `rustwain-default` and loads
  it at agent startup (the agent embeds the profile text; loading needs
  `apparmor_parser` binary *or* writing the compiled policy to
  `/sys/kernel/security/apparmor/.replace` – the latter requires compiling policy, which
  means depending on the parser; `[open]`: shell out once at startup to
  `apparmor_parser` vs ship a precompiled binary policy per ABI).
- Ubuntu 23.10+ restricts unprivileged user namespaces through AppArmor; our privileged
  agent is unaffected but rootless mode needs a profile granting `userns`.

### Landlock

Unprivileged, stackable filesystem (5.13+), network bind/connect (6.7+, ABI v4), and
scoped signal/abstract-socket (6.12+, ABI v6) sandboxing. A process restricts *itself*.

Rustwain use `[proposed]`:
- **Sandbox our own components**: the supervisor, the image unpacker (parses untrusted
  tar data!), and the DNS cache restrict themselves to the paths they need.
- **Optional workload layer**: a per-container Landlock ruleset generated from the
  volume mounts (write only to volume paths and `/tmp`). Cheap extra defence where the
  kernel supports it; skipped silently otherwise.
- Crate: `landlock` (official Rust crate maintained by the Landlock author).

### BPF-LSM

`CONFIG_BPF_LSM` + `lsm=...,bpf` on cmdline (not default on many distros). Lets us
attach eBPF programs to LSM hooks for custom policy (e.g. deny `ptrace` across pods,
restrict `bind` to allowed ports). `[open]` – powerful but availability is spotty. The
probe reports whether it's active.

## Masked and read-only paths

Inside every container, by default:

- **Masked** (bind-mount `/dev/null` or empty tmpfs over): `/proc/acpi, /proc/kcore,
  /proc/keys, /proc/latency_stats, /proc/timer_list, /proc/timer_stats, /proc/sched_debug,
  /proc/scsi, /sys/firmware, /sys/devices/virtual/powercap`.
- **Read-only**: `/proc/asound, /proc/bus, /proc/fs, /proc/irq, /proc/sys,
  /proc/sysrq-trigger`.
- `/sys` mounted read-only; with userns + own netns, sysfs mounts reflect the netns.
- Optional `readOnlyRootFilesystem` – lint warns when not set.

## Privileged containers

"Privileged" in Kubernetes means: all capabilities, no seccomp, no AppArmor/SELinux
confinement, all devices, writable `/sys` and `/proc`. Effectively host root.

Rustwain: keep the concept for system DaemonSets (the CSI node plugin, device plugins,
network tooling) but:
- It can only be granted at the **tenant/namespace policy** level, not per pod freely.
- Prefer fine-grained grants: specific capabilities, specific devices, specific host
  paths. Lint suggests the minimal set using a seccomp/cap **recording mode**.

## Putting it together: the pre-exec sequence

In the container init process (in the supervisor's child, after namespaces and rootfs
are set up):

```
1.  set hostname (uts ns)
2.  apply sysctls (net.* in netns, kernel.shm*/msg*/sem in ipc ns)
3.  apply rlimits (setrlimit / prlimit)
4.  pivot_root done earlier; chdir to workdir (verify inside root!)
5.  set LSM exec label (SELinux setexeccon / AppArmor exec)
6.  setgroups / setresgid / setresuid (KEEPCAPS as needed)
7.  drop bounding caps, capset, raise ambient caps
8.  prctl(NO_NEW_PRIVS)
9.  landlock_restrict_self (optional)
10. close_range(3, ~0) except the sync/exec-error fd (CLOEXEC)
11. seccomp(SECCOMP_SET_MODE_FILTER, TSYNC)   ← last, so earlier steps aren't filtered
12. execve(entrypoint)                        ← on failure, write errno to exec-error fd
```

Seccomp must be last because the filter may block syscalls used in steps 1–10. If the
user profile blocks `execve` itself, the container fails – detectable via the exec-error
fd.

## Open questions

- Ship our own SELinux policy module vs require `container-selinux`? Requiring the
  package is simpler; shipping lets us tighten the policy for our supervisor.
- AppArmor loading without `apparmor_parser`.
- Default for `NET_RAW` – dropping it breaks some diagnostics tools; the ping sysctl
  mitigates most of it.

## Build checklist

- [ ] `sys::caps`, `sys::seccomp` (seccompiler-based), `sys::lsm` (SELinux, AppArmor
      backends with a `NoLsm` fallback), `sys::landlock`.
- [ ] Default seccomp profile per arch, compiled at build time, with a test asserting
      `kexec_load`, `init_module`, `open_by_handle_at` return EPERM.
- [ ] Test that `execve` of a setuid binary does not elevate (NNP).
- [ ] Test matrix: SELinux enforcing host (Fedora), AppArmor host (Ubuntu), no-LSM host
      (Arch/Alpine).
- [ ] `rwctl seccomp record` design (user-notif based recorder).
