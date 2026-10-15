# Sidecars and Debug Containers

**K8s analogue:** native sidecar containers (restartable init containers, KEP-753, GA in
1.33), ephemeral containers (`kubectl debug`, GA in 1.25).

## Sidecars

### The history (what went wrong in Kubernetes)
For years, "sidecars" (service mesh proxies, log shippers, secret agents, cloud SQL
proxies) were just regular containers in the pod. Problems:
- **Startup ordering**: the app started before its proxy was ready → failed initial
  connections. Workarounds: `holdApplicationUntilProxyStarts` hacks, postStart blocking.
- **Shutdown ordering**: on termination, the proxy got SIGTERM simultaneously with the
  app and exited first → the app's in-flight requests failed.
- **Jobs never complete**: a Job's pod with a sidecar never terminates because the sidecar
  runs forever after the main container exits. Workarounds: shared-volume "I'm done"
  files, `curl -X POST localhost:15020/quitquitquit`.
- **Init containers can't use the mesh**: init containers run before regular
  containers, so they lacked the proxy.

Kubernetes 1.28+ fixed this with init containers having `restartPolicy: Always`.

### Rustwain sidecars `[decided]`
Same semantics as Kubernetes native sidecars, designed in from the start, with an
explicit field rather than an overloaded init container:

```yaml
spec:
  sidecars:                     # started in order, before init containers that follow them
    - name: proxy
      image: ...
      startupProbe: { httpGet: { port: 15021, path: /healthz/ready } }
  initContainers:
    - name: migrate             # runs after proxy is started (& startup probe passed)
  containers:
    - name: app
```

A separate `sidecars` field loses interleaving, though (a sidecar that must start *after* some init container,
e.g. after a config-fetch init). Kubernetes' encoding (sidecars inside the
`initContainers` list) captures interleaving naturally. Decision: keep a **single ordered
`init` list** where each entry is either `kind: run-to-completion` or `kind: sidecar`:

```yaml
  init:
    - name: fetch-config        # run to completion
      image: ...
    - name: proxy
      sidecar: true             # starts, stays up; next step waits for its startup probe
      image: ...
    - name: migrate
      image: ...
  containers:
    - name: app
```

Semantics:
- Started in list order. A sidecar step "completes" when it's started (and its startup
  probe passes if defined).
- Sidecars restart on exit regardless of pod restart policy (until pod termination).
- Sidecars' readiness contributes to pod readiness (configurable).
- **Termination**: main containers stopped first; sidecars stopped afterwards in
  **reverse order**, each getting at least a minimum grace slice.
- **Completion**: for `Never`/`OnFailure` pods (Jobs), the pod completes when all main
  containers complete; sidecars are then stopped – fixes the "Job never finishes" problem.
- Resource accounting: sidecars' requests add to the pod's requests (like main
  containers); init (run-to-completion) containers' requests use the "max of init vs
  sum of main+sidecars" rule (Kubernetes' effective request formula).

### Sidecar-less where possible
Many sidecars exist because the platform lacked a feature. Rustwain reduces the need:
- Service mesh mTLS: node-level, no per-pod proxy (see
  [service mesh](../05-networking/10-service-mesh.md)).
- Log shipping: node-level, reading supervisor log files.
- Secret refresh: projected volumes with rotation built in (see
  [config delivery](../08-workloads/05-config-and-secret-delivery.md)).
- Cloud SQL proxies etc.: still sidecars (app-specific).

## Debug (ephemeral) containers

Problem: production images are minimal (distroless, scratch) – no shell, no curl, no
strace. How to debug a running pod?

Kubernetes: `kubectl debug pod/x --image=busybox --target=app` adds an **ephemeral
container** to the running pod: shares the pod's namespaces (net, ipc, uts) and
optionally the target container's PID namespace (`--target`), can't have ports/probes/
resources, can't be removed (stays in the spec as terminated).

Rustwain:
- `rwctl debug pod/x --image=ghcr.io/rustwain/debug-tools --target=app`.
- Implementation: supervisor creates a new container in the pod's existing sandbox with
  `setns` into the target container's PID namespace (if per-container PID ns) and
  **optionally its mount namespace view via `/proc/<target>/root`** (the debug container
  sees the target's filesystem at `/proc/1/root` when sharing PID ns – same as
  Kubernetes).
- Security profile for debug containers is a **separate policy**: may grant
  `SYS_PTRACE` (for strace/gdb) only within the pod's userns – safe-ish due to userns.
  Requires `pods/debug` permission, audit-logged.
- Debug containers are tracked in pod status (not spec) and are removable after they
  exit (Kubernetes' ones linger) `[proposed]`.
- Resource accounting: debug containers run inside the pod cgroup (share the pod's
  limits) – Kubernetes ephemeral containers have no resource fields; same here but
  bounded by the pod cgroup.

### Copy-and-debug
`rwctl debug pod/x --copy --set-image=app=debug-image` creates a copy of the pod (not
behind services) with modifications – useful for crash-looping pods. Same as Kubernetes.

### Node and VM debugging
- Node: see [logs/exec](06-logs-exec-attach-port-forward.md) (`rwctl debug node`).
- MicroVM pods: debug container runs **inside the guest** via guest agent.

## Open questions

- Unified `init` list vs separate `sidecars` field – unified chosen for interleaving;
  revisit if it confuses users.
- Should sidecars be able to declare "I must be ready before app *and* after app stops"
  separately for startup and shutdown? Reverse-order shutdown covers known cases.

## Build checklist

- [ ] Ordered init list with sidecar entries; ordering engine tests (incl. Jobs completing).
- [ ] Reverse-order sidecar termination with minimum grace.
- [ ] Ephemeral debug containers with target PID sharing and separate security policy.
- [ ] `rwctl debug --copy`.
