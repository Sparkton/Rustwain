# Process Supervision

**K8s analogue:** containerd-shim (`containerd-shim-runc-v2`), the pause container's
reaping role, conmon (CRI-O's monitor), tini/dumb-init.

Someone has to be the parent of every container process: wait for it to exit, collect
its exit status, hold its stdio, forward signals, and do all of this even while the node
agent is restarting. That is the **supervisor**.

## Why a separate supervisor process at all

If the agent were the direct parent of container processes:
- Restarting or upgrading the agent would kill or orphan every container (children are
  reparented to init/subreaper and their exit status is lost to us).
- An agent crash (a bug in any of its many modules) would take down workloads.

Kubernetes solved this with containerd-shim per pod (~5–10 MB RSS each, Go). CRI-O uses
conmon (C, ~1 MB). Rustwain's supervisor is Rust, single-threaded, synchronous,
statically linked, target **< 1 MB RSS**.

## Linux mechanisms

### Parent/child and reaping
- When a process exits, it becomes a **zombie** until its parent calls `wait*()`.
- If the parent dies first, children are reparented to the nearest **subreaper**
  ancestor (`prctl(PR_SET_CHILD_SUBREAPER, 1)`), or to PID 1.
- Supervisor sets itself as a subreaper so double-forked daemons inside a container
  (when there's no PID ns) still come back to it.

### pidfd
- `pidfd_open(pid)` / `CLONE_PIDFD` gives an fd that refers to *this* process, immune to
  PID reuse.
- `pidfd_send_signal(fd, sig)`: race-free signalling.
- `poll()` on a pidfd becomes readable when the process exits – integrates with an event
  loop without `SIGCHLD` handler gymnastics.
- `waitid(P_PIDFD, fd, …)` (5.4+) to reap.
- `pidfd_getfd` (5.6+) can duplicate an fd from another process (with ptrace rights) –
  useful for recovering the pty master after supervisor-side restarts `[open]`.

### Signals
- `SIGCHLD` handling via `signalfd` (block signals, read them as fd events) – avoids
  async-signal-safety problems entirely.
- PID 1 in a PID namespace ignores signals it has no handler for (except SIGKILL/SIGSTOP
  from an ancestor namespace). So `SIGTERM` to a container whose entrypoint is a naive
  shell script does nothing → the pod waits the whole grace period and then gets
  SIGKILLed. Mitigations: `init: true` (inject `rustwain-init`), and clear reporting in
  status: "container ignored SIGTERM; killed after 30s".

### The PID 1 problem inside containers
An app running as PID 1 must:
1. Reap orphaned zombies (otherwise PID exhaustion).
2. Install handlers for SIGTERM/SIGINT.

`rustwain-init` (like tini): a ~200 KB static binary that forks the real entrypoint,
forwards signals to it (optionally to its whole process group), reaps everything, and
exits with the child's status. Bind-mounted read-only from the host into the container
at `/.rustwain/init` – no image modification.

## Supervisor design

### Lifecycle
1. Agent decides to start a pod. It writes a fully resolved **pod execution spec**
   (`/run/rustwain/pods/<uid>/spec.bin`) – every path, every ID mapping, every seccomp
   program already compiled. The supervisor does **no policy decisions**.
2. Agent spawns the supervisor with `clone3(CLONE_INTO_CGROUP=<pod>/supervisor)` +
   `execve("/usr/local/bin/rustwain", ["rustwain", "supervise", uid])`. The supervisor
   immediately `setsid()`s and **double-checks its parent is not needed** (it does not
   depend on the agent staying alive).
3. Supervisor creates the sandbox (namespaces – see [namespaces](01-namespaces.md)),
   then opens its control socket `/run/rustwain/pods/<uid>/ctl.sock` and signals
   readiness.
4. Agent connects, and issues commands: `StartContainer(name)`, `Signal`, `Exec`,
   `Attach`, `Stats`, `Shutdown`, `UpdateResources`.
5. Supervisor emits events: `ContainerStarted{pid}`, `ContainerExited{status, oom}`,
   `ExecExited`, `LogRotated`.
6. When every container has exited and the agent says `Shutdown`, the supervisor
   cleans up mounts/namespaces and exits.

### State and recovery
- Supervisor writes a small **state file** (atomic rename) after each transition:
  container PIDs (as recorded at start), start times, exit codes not yet acknowledged by
  the agent.
- **Agent restart:** new agent scans `/run/rustwain/pods/*/ctl.sock`, connects, sends
  `Hello{agent_epoch}`, receives the full state and any unacknowledged events. Events are
  acknowledged by sequence number so none are lost.
- **Supervisor crash** (should never happen; it's tiny): containers are children of the
  supervisor; on its death they are reparented to the next subreaper – the **agent**
  is also a subreaper, so it inherits them and can at least reap and report. The pod is
  then marked `Degraded` and restarted at the next safe opportunity (policy).
- **Host reboot:** everything in `/run` is gone; the agent rebuilds from desired state.

### Event loop
Single-threaded `poll()` (or `epoll`) over: control socket listener + clients, one pidfd
per container/exec process, signalfd, pty masters and log pipes, inotify (none
needed). No tokio. Code size target: < 3,000 lines.

### Stdio and logs
- Each container's stdout/stderr are pipes (or a pty if `tty: true`) owned by the
  supervisor.
- Supervisor writes log records to `/var/log/rustwain/pods/<ns>_<pod>_<uid>/<container>/N.log`
  in a framed format (timestamp, stream, partial flag, bytes) and rotates by size.
- Why the supervisor and not the agent: logs must keep flowing during agent restarts,
  and pipes must have a reader or the container blocks on write once the pipe buffer
  (64 KiB default) fills.
- Optimisation `[proposed]`: use `splice`/`tee` to move data pipe → file without copying
  through userspace when no parsing is needed (we need framing, so we frame per read and
  use `writev`).
- Back-pressure: if the disk is full, drop with a counter rather than block the
  container (configurable: `block` for audit-critical workloads).

See [logs/exec/attach](../04-node-agent/06-logs-exec-attach-port-forward.md).

### Exit status and OOM detection
- Exit code via `waitid`; signal number if killed.
- OOM: watch `memory.events` `oom_kill` counter of the container's cgroup (inotify on
  the file or read on exit). If it incremented during the container's life, report
  `reason: OOMKilled` – accurate even when the OOM-killed process was a child, not the
  main process (Kubernetes historically misreported this).

## Supervisor for VM pods

For `microvm`/`vm` isolation classes, the supervisor is the parent of the **VMM**
process (or *is* the VMM, `[open]`). It communicates with `rustwain-guest` inside the VM
over vsock, which performs the same container-start logic inside the guest. The agent
sees the same control protocol either way. See
[KVM and microVMs](../03-virtualization/02-kvm-and-microvms.md).

## Alternatives considered

| Option | Why not |
|---|---|
| Agent is direct parent (no supervisor) | Agent restarts kill workloads |
| One node-wide supervisor daemon for all pods (Docker's original model) | Single point of failure; a bug hits every pod; harder to account per pod |
| Use systemd transient units as supervisors | Ties us to systemd; D-Bus latency per pod; journald for logs is slow at high volume |
| Reuse conmon | C, not Rust; protocol designed for CRI-O |

## Open questions

- Should the supervisor be the same binary (`rustwain supervise`) or a separate tiny
  binary? Same binary is simpler to distribute, but the agent binary is larger (more
  pages to map, though only touched pages count toward RSS). Measure.
- Supervisor upgrades: we can't hot-upgrade supervisors. Protocol must be versioned and
  the agent must speak N-1 and N supervisor protocol versions.

## Build checklist

- [ ] `rustwain-supervisor` crate: no tokio, no allocations in fork path, `poll` loop.
- [ ] `rustwain-init` crate: tini-equivalent, static, tiny.
- [ ] Versioned control protocol with sequence-numbered events and ack.
- [ ] Integration test: kill -9 the agent mid-start, restart, verify no lost exit events.
- [ ] Integration test: zombie reaping, SIGTERM forwarding with and without `init: true`.
- [ ] RSS budget test: supervisor with 1 container < 1 MB RSS.
