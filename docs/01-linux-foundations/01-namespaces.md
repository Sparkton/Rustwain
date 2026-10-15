# Linux Namespaces

**K8s analogue:** what runc does under the hood for every container, and what the
"pause" container exists to hold open.

A container is not a kernel object. It is an ordinary process placed into a set of
**namespaces** (what it can see), a **cgroup** (what it can use), and a set of
**security restrictions** (what it can do), with its root filesystem swapped. This
document covers the "what it can see" part.

## The eight namespace types

| Namespace | Flag | Isolates | Kernel | Notes for Rustwain |
|---|---|---|---|---|
| Mount | `CLONE_NEWNS` | Mount table | 2.4.19 | Every container gets its own; propagation must be set carefully |
| UTS | `CLONE_NEWUTS` | hostname, domainname | 2.6.19 | Per pod (shared by containers in a pod) |
| IPC | `CLONE_NEWIPC` | SysV IPC, POSIX mqueues | 2.6.19 | Per pod |
| PID | `CLONE_NEWPID` | PID numbers | 2.6.24 | Per pod by default, optional per container |
| Network | `CLONE_NEWNET` | Interfaces, routes, iptables/nft, sockets, `/proc/net` | 2.6.29 | Per pod; the dataplane wires it |
| User | `CLONE_NEWUSER` | UIDs/GIDs, capabilities scope | 3.8 (usable ~4.x; idmapped mounts 5.12+) | **On by default** in Rustwain |
| Cgroup | `CLONE_NEWCGROUP` | View of `/sys/fs/cgroup` | 4.6 | Per container so it sees its own subtree as root |
| Time | `CLONE_NEWTIME` | `CLOCK_MONOTONIC`/`BOOTTIME` offsets | 5.6 | Only for checkpoint/restore migration; not used by default |

## The syscalls

- **`clone3(struct clone_args)`** (5.3+): create a new process directly in new
  namespaces. Supports `CLONE_INTO_CGROUP` (5.7+) to place the child into a cgroup
  atomically at birth – this avoids a race where a process briefly runs in the parent
  cgroup and escapes accounting. Also returns a **pidfd** (`CLONE_PIDFD`).
- **`unshare(flags)`**: move the *calling* process into new namespaces. For PID
  namespaces, only *children* created afterwards enter the new namespace.
- **`setns(fd, nstype)`**: join an existing namespace by fd (`/proc/<pid>/ns/net`). Since
  5.8, `setns` accepts a **pidfd** and a flag set, joining several namespaces of that
  process atomically – ideal for `exec` into a running container.
- **`ioctl(NS_GET_*)`**: query relationships (parent user namespace, owner UID).
- **`/proc/<pid>/ns/*`**: namespace handles; bind-mounting one keeps the namespace alive
  without a process (how CNI plugins traditionally hold netns).

Decision `[decided]`: **Use `clone3` with `CLONE_INTO_CGROUP | CLONE_PIDFD` as the only
process-creation path for container init.** Kernels without `clone3` (< 5.3) are below
our minimum anyway.

## Ordering problems you must get right

The order in which namespaces are created and joined matters, and getting it wrong
produces subtle bugs or security holes.

1. **User namespace first.** Creating other namespaces requires `CAP_SYS_ADMIN` *in the
   owning user namespace*. If you create the user namespace first, the new process has
   full capabilities inside it and can create the rest unprivileged. Every other
   namespace created afterwards is **owned** by that user namespace, which is what makes
   it safe: capabilities in the container's userns do not apply to host-owned resources.
2. **UID/GID maps must be written by the parent** (from outside) after the child is
   created and before it does anything that depends on identity. The standard pattern
   is a sync pipe: child blocks on read, parent writes `/proc/<child>/uid_map`,
   `/proc/<child>/gid_map` (and `setgroups` = `deny` when required), then releases the
   child.
3. **PID namespace: the first process is PID 1.** It gets init semantics: signals
   without handlers are ignored by default, orphans are reparented to it, and when it
   exits **the kernel kills every other process in the namespace**. See
   [process supervision](06-process-supervision.md).
4. **Mount namespace and `pivot_root`.** After entering the new mount namespace, make
   the whole tree private (`mount --make-rprivate /`) *before* any mounts, or your
   container mounts propagate back to the host. Then bind-mount the rootfs onto itself,
   set up `/proc`, `/dev`, `/sys`, then `pivot_root(new_root, put_old)` and unmount
   `put_old` with `MNT_DETACH`. Never use `chroot` for containers – it is escapable.
5. **`/proc` must be mounted after entering the PID namespace** and from a process
   that is in it, otherwise it shows the host's PIDs.
6. **Network namespace** can be created by the supervisor before any container starts,
   so the dataplane can configure interfaces and the IP is known before containers run.

## How a pod uses namespaces

```
Pod sandbox (held by rustwain-supervisor)
├─ user ns  (shared by all containers in the pod)   ← one ID mapping per pod
├─ net ns   (shared)                                ← one IP, one loopback
├─ uts ns   (shared)                                ← one hostname
├─ ipc ns   (shared)                                ← containers can use shm together
├─ pid ns   (shared by default; option: per container)
│
├─ container "app"
│   ├─ mount ns (own)  ← own rootfs, own volume mounts
│   └─ cgroup ns (own) ← sees /sys/fs/cgroup as its own subtree
└─ container "sidecar"
    ├─ mount ns (own)
    └─ cgroup ns (own)
```

### No pause container

Kubernetes runs a "pause" container per pod whose only job is to hold the shared
namespaces open (and, with a shared PID namespace, act as PID 1 to reap zombies).

Rustwain's **supervisor holds namespace file descriptors** instead. A namespace stays
alive as long as any fd references it, so the supervisor opens
`/proc/self/ns/{net,ipc,uts,user}` of the first process it creates in the pod (or uses
`unshare` + fds directly) and keeps them. Containers join with `setns`.

When the pod uses a **shared PID namespace**, something must be PID 1 inside it. Options:

| Option | Pros | Cons |
|---|---|---|
| Supervisor itself enters the PID namespace and forks a tiny init as PID 1 | One fewer image to pull, reaps zombies | Supervisor has a process inside the pod's PID ns |
| A static `rustwain-init` binary (a few hundred KB) bind-mounted in | Isolated from supervisor | Still an extra process |
| No shared PID ns by default; each container's own entrypoint is PID 1 | Simplest | Zombie reaping is up to the app (the classic "PID 1 problem") |

Decision `[proposed]`: **shared PID namespace off by default (matches Kubernetes);
when on, a tiny static `rustwain-init` is PID 1.** When off, the container's entrypoint
is PID 1 but runs with the `PR_SET_CHILD_SUBREAPER` supervisor as its real parent
outside the namespace; we also offer `init: true` to inject `rustwain-init` per
container (like `docker run --init`).

## Network namespace specifics

- A new netns contains only a `lo` interface, **down**. The runtime must bring it up.
- Sysctls under `net.*` are mostly per-netns (e.g. `net.ipv4.ip_local_port_range`,
  `net.core.somaxconn`). This is how "safe sysctls" per pod work. Some (`net.core.rmem_max`
  before certain kernels, `net.netfilter.nf_conntrack_max`) are global or partially global
  – the [host variance](05-linux-host-variance.md) probe must know which.
- Moving an interface into a netns: create a veth/netkit pair in the host ns, move one
  end with `IFLA_NET_NS_FD`. Do this over **netlink** (rtnetlink) from Rust – the
  `rtnetlink`/`netlink-packet-route` crates or our own thin wrapper – never by shelling
  out to `ip`.
- Holding a netns: either an fd in the supervisor (preferred) or a bind mount at
  `/run/rustwain/netns/<pod-uid>` (needed if external tools like CNI plugins must find it
  by path).

## User namespace specifics

Covered in depth in [rootless and user namespaces](../02-container-runtime/04-rootless-and-userns.md).
Key facts:

- A process with UID 0 inside a userns maps to an unprivileged UID outside.
- Capabilities granted inside only apply to resources owned by that userns.
- Files on disk carry host UIDs; **idmapped mounts** (5.12+, per-filesystem support
  grew through 5.19–6.x: ext4, xfs, btrfs, overlayfs on 5.19+, tmpfs on 6.3+) let us
  present image layers with shifted ownership without copying/chowning.
- Some sysctls and kernel features behave differently or are blocked under userns:
  e.g. unprivileged userns can be disabled by distro sysctls
  (`kernel.unprivileged_userns_clone` on Debian-patched kernels, `user.max_user_namespaces`,
  Ubuntu 24.04's AppArmor restriction `kernel.apparmor_restrict_unprivileged_userns`).
  The agent runs privileged, so these mostly matter for rootless mode, but the probe
  records them.

## Mount namespace specifics

Covered in [filesystems](04-filesystems.md). The essentials:

- Propagation types: `shared`, `slave`, `private`, `unbindable`. systemd makes `/`
  **shared** by default, so new mount namespaces inherit shared propagation – always
  re-mark as private/slave first.
- Volumes that need mounts to propagate *from the host into the container* later (e.g.
  CSI drivers mounting FUSE filesystems) use `rslave`; volumes that need propagation *out
  of* a privileged container (storage drivers) use `rshared` and require privilege.
- The new mount API (`fsopen`, `fsconfig`, `fsmount`, `move_mount`, `open_tree`,
  `mount_setattr`) in 5.2+ (idmap via `mount_setattr` 5.12+) is fd-based and avoids
  path races. **Rustwain uses the new mount API exclusively** when the kernel supports
  it, falling back to `mount(2)` only for legacy cases.

## Cgroup namespace

When a process in a cgroup namespace reads `/proc/self/cgroup` or mounts cgroupfs, it
sees its own cgroup as `/`. This hides the host layout (avoiding information leaks) and
lets nested systemd inside a container work. We always create one per container, after
placing the process into its cgroup (the namespace root is the cgroup at creation time).

## Time namespace

Lets a container see offset monotonic/boottime clocks. Only useful for
**checkpoint/restore** (CRIU) live migration so restored processes don't see time jump.
Not wall-clock (`CLOCK_REALTIME` cannot be namespaced). Parked for the migration work
in [full VM workloads](../03-virtualization/04-full-vm-workloads.md).

## Joining a running pod (exec)

For `rwctl exec`:

1. Agent asks the supervisor (which holds the pod's pidfds) to spawn.
2. Supervisor forks a helper which calls `setns(pidfd_of_container_init,
   CLONE_NEWUSER|CLONE_NEWNS|CLONE_NEWPID|CLONE_NEWNET|CLONE_NEWIPC|CLONE_NEWUTS|CLONE_NEWCGROUP)`
   (single atomic call, 5.8+), then forks again (needed to actually be *in* the PID ns),
   moves into the container's cgroup, applies the same seccomp/caps/LSM profile, and
   `execve`s the requested command.
3. The exec'd process must get the **same security restrictions as the container** –
   historically a source of CVEs (e.g. runc CVE-2019-5736 overwrote the host runc
   binary via `/proc/self/exe` during exec; mitigated by executing from a sealed memfd
   copy of the binary, or – now preferable – by not re-executing the runtime binary
   inside the container at all).

## Pitfalls checklist

- [ ] Forgot `make-rprivate` → container mounts leak to host.
- [ ] Mounted `/proc` before entering PID ns → host PIDs visible.
- [ ] Wrote uid_map after the child already exec'd → capability confusion.
- [ ] Left an fd to a host directory open across `pivot_root` → escape (runc
      CVE-2024-21626 "Leaky Vessels" leaked a host cwd fd). Rule: **close every fd not
      explicitly passed, with `close_range(3, ~0, CLOSE_RANGE_CLOEXEC)` before exec**, and
      verify cwd is inside the new root.
- [ ] Used `chroot` → escapable.
- [ ] Ran PID 1 without a signal handler → `SIGTERM` ignored, pod waits full grace period.
- [ ] Bind-mounted netns path not cleaned up → leaked namespaces after crash. Track in
      the checkpoint and GC on startup.

## Rust implementation notes

- `rustix` covers most of it: `rustix::thread::{unshare, move_into_link_name_space}`,
  `rustix::mount::*` (including the new mount API), `rustix::process::pidfd_open`.
  `clone3` is not wrapped safely anywhere because forking in a multithreaded Rust
  process is dangerous; we write a tiny `unsafe` wrapper used **only** from the
  single-threaded supervisor.
- Never `fork` from a tokio process. The agent asks the supervisor (single-threaded,
  synchronous) to do all process creation. The supervisor itself is started via
  `posix_spawn`-like `clone3` + `execve` from the agent – exec immediately after fork
  is safe.
- After `clone3` in the child, only async-signal-safe operations until `execve`: no
  allocation, no locks. Pre-compute everything (paths as `CString`, seccomp BPF
  program bytes, capability sets) before cloning.

## Open questions

- Per-container vs per-pod PID namespace default – revisit after user feedback.
- Do we support `hostNetwork` / `hostPID` / `hostIPC`? Yes for system DaemonSets, gated
  behind the `privileged` security level.

## Build checklist

- [ ] `sys::ns` module: typed `NamespaceSet` bitflags, fd holders, `setns` by pidfd.
- [ ] `sys::clone` minimal `clone3` wrapper with `CLONE_INTO_CGROUP` and `CLONE_PIDFD`.
- [ ] Sync-pipe protocol for uid/gid map writing.
- [ ] Integration test: create every namespace type, verify isolation from inside via
      `/proc/self/ns/*` inode comparison.
- [ ] Integration test: fd leak detector – exec'd process must only have fds 0,1,2.
