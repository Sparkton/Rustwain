# Logs, Exec, Attach, Port-Forward and Copy

**K8s analogue:** `kubectl logs/exec/attach/port-forward/cp`, kubelet streaming server,
CRI streaming, container log rotation (`containerLogMaxSize`, `containerLogMaxFiles`),
CRI log format.

## Container logs

### Format on disk
Kubernetes CRI log format is a text line per record: `<RFC3339Nano> <stream> <P|F> <content>`.
Simple, but: parsing timestamps is slow, partial lines are messy, binary output gets
mangled.

Rustwain on-disk format `[proposed]`: binary framed records:

```
u32 length | u64 timestamp_ns | u8 stream (stdout/stderr) | u8 flags (partial) | bytes
```

+ a sparse **time index** (every N KiB, the file offset and timestamp) so
`--since=10m` seeks directly rather than scanning. `rwctl logs` renders text; a
compatibility exporter can write CRI format for existing log shippers (Fluent Bit,
Vector) `[proposed]`.

### Path layout
`/var/log/rustwain/pods/<namespace>_<pod>_<uid>/<container>/<restartCount>.log`
(+ rotated `.log.<n>`, compressed `.zst` after rotation).

- Previous container instance logs kept (for `logs --previous`).
- Written by the **supervisor** (survives agent restarts).

### Rotation and limits
- Per-container max size and file count (default 10 MiB × 5).
- **Per-pod log rate limit** `[proposed]` (lines/s and bytes/s token bucket) – a pod
  spamming logs at 100 MB/s can saturate disk IO for the whole node. Excess is dropped
  with a counter and a marker record ("dropped 12,345 records"). Kubernetes has no such
  limit.
- Log disk usage counts toward the pod's ephemeral storage (Kubernetes counts it too).
- Rotation must not lose data: supervisor rotates by renaming and reopening (it owns the
  fd), no `copytruncate`.

### Streaming `rwctl logs -f`
```
rwctl → API (authz: pods/log get) → API proxies stream to agent → agent tails file
       (inotify / or subscribes to supervisor's live stream for zero latency)
```
- API → agent connection: the API server dials the agent's stream port with mTLS (agent
  verifies the server cert; server verifies node cert matches the pod's node).
- Alternative design `[open]`: **agent dials out** (reverse tunnel) – servers never need to
  reach nodes directly, which helps when nodes are behind NAT (edge) – the agent keeps a
  persistent HTTP/2 connection to the API, and stream requests are multiplexed over it
  (like Kubernetes' Konnectivity proxy, but built in). Strong lean: **agent dial-out
  tunnel**, because it removes a whole category of firewall/NAT problems and doesn't
  require exposing an agent port.

## Exec and attach

### Protocol
Kubernetes uses SPDY (deprecated) and WebSocket (`v5.channel.k8s.io`, default since 1.31)
with channels for stdin, stdout, stderr, error, resize. Rustwain:
- Client ↔ API: HTTP/2 bidirectional gRPC stream with typed frames
  (`Stdin(bytes)`, `Stdout(bytes)`, `Stderr(bytes)`, `Resize{cols, rows}`, `Exit{code}`,
  `Error{...}`). Also a WebSocket variant on the JSON gateway for scripting clients.
- API ↔ agent: same frames over the dial-out tunnel.
- Agent ↔ supervisor: unix socket; supervisor spawns the exec process (namespaces join as
  described in [namespaces](../01-linux-foundations/01-namespaces.md)) with a pty or pipes.

### Security
- Authorization: `pods/exec create` permission. Exec is effectively full access to the
  container – audit-logged with command and user.
- The exec'd process gets the **container's** security profile (seccomp, caps, LSM,
  cgroup) – never more. Ephemeral debug containers can have a different profile (see
  [sidecars and debugging](08-sidecars-and-debugging.md)).
- Session recording `[later]`: optional ttyrec-like capture of exec sessions for
  compliance (stored with audit log).

### Attach
Connect to the main process's stdio (requires `stdin: true`/`tty: true` in spec for
input). The supervisor already owns the pty/pipes, so attach is a multiplexed reader.

## Port-forward

`rwctl port-forward pod/x 8080:80`:
- Kubernetes implements this by having the runtime (containerd) enter the pod netns and
  `connect()` to localhost:port, copying bytes.
- Rustwain: agent opens a socket inside the pod's netns (`setns` on a dedicated thread, or
  better – create the socket in a helper that is in the netns and pass the fd back via
  `SCM_RIGHTS`; sockets keep their netns after creation, so the agent can then use the fd
  from its normal tokio runtime). Bytes copied with `splice` where possible.
- Multiplexed over the same tunnel.
- Also `rwctl port-forward svc/x` – resolves a ready endpoint.

## Copy (`rwctl cp`)

Kubernetes implements `kubectl cp` by exec'ing `tar` inside the container – fails on
images without tar (distroless!) and has had path-traversal CVEs on the client side
(CVE-2019-1002101 etc.).

Rustwain: a native **file API** `[proposed]`:
- Agent (via supervisor helper inside the container's mount ns, with the container's
  userns credentials) reads/writes files directly – no dependency on image contents.
- Streams a tar produced by the helper (Rust `tar` crate) – client extracts with strict
  path sanitisation.
- Requires `pods/files` permission (separate from exec; read-only variant possible).

## Debugging nodes

`rwctl debug node/<n>`: starts a privileged debug pod on the node with the host
filesystem mounted at `/host` (Kubernetes' `kubectl debug node/...`). Gated by a high
privilege permission and audit-logged.

## Open questions

- Agent dial-out tunnel: one connection to one server, or to all? Stream requests must
  reach the server that holds the tunnel → servers need to route among themselves
  (server-to-server proxying) or the client is redirected. Simplest: agents connect to all
  servers (3–5 connections), each server can reach every agent locally.
- Binary log format adoption friction with existing log shippers.

## Build checklist

- [ ] Supervisor log writer with framing, time index, rotation, rate limiting.
- [ ] `rwctl logs` (since, tail, follow, previous, timestamps, all containers).
- [ ] Agent dial-out tunnel (HTTP/2) with stream multiplexing.
- [ ] Exec/attach with pty resize.
- [ ] Port-forward via netns socket fd passing.
- [ ] Native file copy API.
