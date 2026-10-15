# The Low-Level Container Start Sequence

**K8s analogue:** `runc create` + `runc start`, `runc init`, crun, youki's libcontainer.

This is the exact sequence the supervisor performs to turn a `ResolvedPodSpec` into
running processes. It is the most security-sensitive code in the project, so it is
written out step by step, with the reason for each ordering constraint.

## Phase 0: preconditions (done by agent before spawning supervisor)

- Image layers unpacked and present in the snapshot store.
- Container rootfs snapshot prepared (overlay upper/work dirs created, empty).
- Pod cgroup and per-container leaf cgroups created with resource values written.
- UID/GID range allocated for this pod (e.g. 65536 IDs starting at host 1,000,000 +
  N×65536).
- IP allocated; dataplane ready to wire a netns.
- Volumes attached/mounted at host staging paths.
- `spec.bin` written; seccomp program bytes compiled.

## Phase 1: sandbox creation (supervisor)

```
supervisor (host ns, single-threaded)
  │
  ├─ 1. open spec, validate version
  ├─ 2. prctl(PR_SET_CHILD_SUBREAPER)
  ├─ 3. create sync socketpair (SOCK_SEQPACKET, CLOEXEC)
  ├─ 4. clone3(flags = CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWIPC | CLONE_NEWUTS
  │             [| CLONE_NEWPID if shared pid ns],
  │          cgroup = <pod>/sandbox, pidfd = &sbfd)
  │       │
  │       └─ child "sandbox holder":
  │             a. wait on sync socket for "maps written"
  │             b. (now root in its userns) set hostname
  │             c. bring lo up (netlink in its own netns)
  │             d. apply ipc/net sysctls
  │             e. send "ready", then pause() forever  ← cheap: ~100 KB RSS, no image
  │
  ├─ 5. write /proc/<child>/setgroups ("deny" only if mapping requires), uid_map, gid_map
  ├─ 6. signal "maps written"
  ├─ 7. open /proc/<child>/ns/{user,net,ipc,uts[,pid]} → hold fds
  ├─ 8. tell agent: sandbox ready, netns fd/path
  │       agent dataplane: create netkit/veth pair, move peer into netns, assign IP,
  │       routes, attach BPF programs; DNS config written.
  └─ 9. wait for StartContainer commands
```

Note on the "sandbox holder": a namespace stays alive while an fd refers to it, so
strictly no process is needed for user/net/ipc/uts. A process **is** needed for a shared
PID namespace (PID 1). Decision `[proposed]`: when PID ns is shared, the holder becomes
`rustwain-init` (exec'd) as PID 1; when not shared, the holder exits after step 7 and
only fds are held. Fewer processes, less memory.

## Phase 2: container creation

For each container (init containers sequentially, then regular containers per ordering
rules – see [pod lifecycle](../04-node-agent/02-pod-lifecycle.md)):

```
supervisor
  ├─ 1. prepare per-container data:
  │      - argv/envp as CStrings
  │      - mount plan (ordered list of fully resolved mount operations)
  │      - cap sets, seccomp program, LSM label, rlimits, uid/gid/groups
  │      - exec-error pipe (CLOEXEC)
  ├─ 2. pre-fork: open the rootfs dir fd, layer dirs, volume source fds
  │      (opened from the host side, with openat2 RESOLVE_NO_SYMLINKS, before entering
  │       the container's userns, so they reference the intended objects)
  ├─ 3. clone3(flags = CLONE_NEWNS | CLONE_NEWCGROUP [| CLONE_NEWPID if per-container],
  │           cgroup = <pod>/c-<name>, pidfd = &cfd)
  │      BUT first join the pod's namespaces:
  │      → actually: fork an intermediate that setns(user, net, ipc, uts, [pid]) via the
  │        held fds, THEN clone3 the container init with NEWNS|NEWCGROUP. (setns into a
  │        user ns requires single-threaded caller – the supervisor is single-threaded,
  │        but we keep the supervisor itself in host namespaces, hence the intermediate.)
  │
  └─ container init (inside all namespaces, fresh mount ns):
        a.  mount --make-rslave / (or rprivate) recursively
        b.  build rootfs:
              - fsopen("overlay"), fsconfig lowerdir+ for each layer, upperdir, workdir,
                volatile → fsmount → move_mount onto <rootfs>
              - (with userns) idmap the mount: mount_setattr(MOUNT_ATTR_IDMAP, userns_fd)
                  — or idmap lower layers before overlay creation (kernel-dependent)
        c.  mount /proc (new procfs, hidepid=invisible? optional subset=pid)
        d.  mount /dev tmpfs, bind device nodes, devpts newinstance, /dev/shm, mqueue
        e.  mount /sys ro (sysfs, or bind with rbind ro if userns prevents fresh sysfs)
        f.  bind volumes in order (sorted by destination depth so parents come first),
            each target resolved with openat2(RESOLVE_IN_ROOT) relative to rootfs fd;
            create missing mountpoints as dirs/files owned appropriately
        g.  bind /etc/hosts, /etc/hostname, /etc/resolv.conf, secrets tmpfs
        h.  apply masked paths and read-only paths
        i.  make rootfs read-only if requested (remount bind ro — after all mounts!)
        j.  pivot_root(rootfs, rootfs) trick: fchdir(rootfs); pivot_root(".", ".");
            umount2(".", MNT_DETACH)  ← avoids needing a put_old directory
        k.  chdir(workdir) — and verify via getcwd that it is inside the new root
        l.  setrlimits
        m.  LSM exec label
        n.  setgroups, setresgid, setresuid
        o.  capability sets, ambient caps
        p.  prctl(PR_SET_NO_NEW_PRIVS)
        q.  landlock_restrict_self (optional)
        r.  close_range(3, ~0U, CLOSE_RANGE_CLOEXEC) keeping only exec-error fd
            (and any explicitly passed fds, e.g. systemd socket activation LISTEN_FDS)
        s.  seccomp(SECCOMP_SET_MODE_FILTER, SECCOMP_FILTER_FLAG_TSYNC, prog)
        t.  execve(path, argv, envp)
            → on failure: write errno to exec-error fd, _exit(127)
```

Supervisor side, after clone:
- Reads the exec-error pipe: EOF (closed by successful exec via CLOEXEC) = success; data
  = errno → report `StartError { errno, stage }` with a human message
  ("executable not found in image: /app/server", "permission denied: entrypoint not
  executable").
- Records pidfd, registers it in the poll loop.
- Reports `ContainerStarted { pid, started_at }` to agent.

### Why `create` and `start` aren't separate here

The OCI spec splits `create` (set everything up, stop before exec) and `start` (exec).
runc implements the pause with a FIFO. Kubernetes doesn't really use the split for
anything important. Rustwain doesn't need it: the pre-start hooks that justify it
(CNI, OCI hooks) are done before cloning. The one use is **startup-timing measurement**
and **postStart** hooks, which we handle at the supervisor level.

## Phase 3: running

- Supervisor polls pidfds, stdio pipes, control socket.
- Exec requests: see [namespaces – joining a running pod](../01-linux-foundations/01-namespaces.md).
- Resource updates (in-place resize): agent writes cgroup files directly (supervisor not
  involved) – cgroup writes are not namespace-dependent.

## Phase 4: stopping

1. Agent sends `Stop(container, grace)`.
2. If `preStop` hook defined: run it (exec or HTTP) first, counting against grace.
3. Send the container's stop signal (`STOPSIGNAL` from image or spec override; default
   SIGTERM) via `pidfd_send_signal` to PID 1 of the container (or to the process group
   if configured).
4. Wait for pidfd readiness up to the remaining grace.
5. On timeout: write `1` to the container's `cgroup.kill` → atomically SIGKILLs every
   process in the cgroup, including racing forks.
6. Reap, record exit status and OOM state.
7. Unmount nothing explicitly – the container's mount namespace dies with its last
   process, and all mounts in it go away (lazy). Overlay upper dir is removed by the agent
   (or kept for `rwctl debug` post-mortem if configured).

## Error handling and cleanup

Every resource created gets a **cleanup record** written *before* creation in the pod
state dir (intent log): "about to create cgroup X", "about to mount Y". On any failure
or crash, cleanup replays the intent log in reverse and removes what exists. This is how
we avoid leaking cgroups, netns bind mounts, overlay mounts and veths — the classic
"orphaned pause/netns after kubelet crash" problems.

## Security invariants (tested)

1. No fd other than 0/1/2 (and declared passthrough fds) is open in the container at
   exec time.
2. The container cannot see any path outside its rootfs, volume mounts, and the
   standard pseudo-filesystems.
3. Mount targets are resolved inside the rootfs even if the image contains malicious
   symlinks (`/etc -> /`, `/proc -> /tmp/fake`).
4. `/proc/self/exe` in the container points to the entrypoint, never to a host binary
   (we never execute a host binary inside the container's namespaces after pivot, except
   `rustwain-init` which is bind-mounted read-only and sealed – and even that is
   executed from a memfd sealed with `F_SEAL_WRITE|F_SEAL_GROW|F_SEAL_SHRINK|F_SEAL_SEAL`
   to prevent the CVE-2019-5736 class of overwrite).
5. The container's capability bounding set never exceeds the spec.
6. seccomp is installed with TSYNC before exec; `/proc/<pid>/status` shows `Seccomp: 2`.
7. `NoNewPrivs: 1` unless explicitly privileged.

## Performance notes

- Pre-compile everything; the clone→exec path must be < 5 ms on a warm host.
- Overlay mount with many layers: the new mount API avoids the option-string length
  limit and string parsing.
- Avoid `sync()` anywhere in the hot path.
- Process creation via `clone3` with `CLONE_VFORK`-like semantics is *not* used because
  the child needs to do substantial setup; plain clone is fine — the parent is tiny so
  CoW page-table copying is cheap (another reason the supervisor must be small).

## Open questions

- Idmap the overlay mount vs idmap each lower layer: depends on kernel (overlayfs
  idmapped mounts support since 5.19 maps the *overlay* mount; layers can also be
  idmapped individually). Benchmark both.
- `hidepid=invisible` for /proc in pods with shared PID namespace – breaks some tools.

## Build checklist

- [ ] `rustwain-container` crate implementing phases 1–4 with an intent log.
- [ ] Exec-error pipe with stage reporting.
- [ ] Security invariant tests 1–7 as integration tests (require root, run in CI VMs).
- [ ] Malicious image fixtures (symlink tricks, device nodes, setuid files, huge
      layers, path traversal in tar entries).
- [ ] OCI runtime-tools validation via `rustwain oci`.
