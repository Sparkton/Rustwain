# Audit Logging and Events

**K8s analogue:** audit policy (levels None/Metadata/Request/RequestResponse, stages),
audit backends (log file, webhook), Events (core/v1 and events.k8s.io/v1), event
aggregation/deduplication, `kubectl events`.

## Events

### Purpose
Human-readable, short-lived records of notable things: "Pulled image in 2.3s",
"Back-off restarting failed container", "Scheduled to node-3", "FailedMount: ...".

### Kubernetes problems
- Events are regular objects in etcd (TTL 1 h by default) → event storms (a crash-looping
  DaemonSet on 5,000 nodes) write heavily into etcd; many clusters split events into a
  separate etcd.
- Dedup/aggregation is client-side (in the recorder) and imperfect.
- Events are not a reliable history (dropped under load, short TTL) yet people treat them
  as one.

### Rustwain design `[proposed]`
- **Separate event store** on each server: in-memory ring buffer + optional append-only
  file, **not** in the Raft log. Each server accepts events from clients; events are
  replicated asynchronously to peers best-effort (or not at all – queries fan out to all
  servers and merge) – events are allowed to be lossy.
- **Server-side aggregation**: identical events (same object, reason, message template)
  within a window increment a count instead of creating new records.
- **Rate limits** per source and per object.
- Typed `reason` enums for built-in components (consistent, searchable).
- Retention: by size and time (default 1 h / 1 GiB per server).
- API: `Events` kind with list/watch (served from event stores), `rwctl events --for
  pod/x`.
- Events are inputs to `rwctl describe`/`why`, never to controllers' logic (controllers use
  conditions).

## Audit log

### Purpose
Security-relevant, reliable record of **who did what, when, to which object, from where,
and the outcome**.

### Design
- Every API request passes an audit stage after completion (and at start for long-running
  requests like exec/watch).
- **Policy** (`AuditPolicy` object or config file): rules by user/group/verb/resource/
  namespace → level `None | Metadata | Request | RequestResponse`. Defaults:
  - `Metadata` for everything,
  - `None` for high-volume read-only system traffic (agent watches, lease renewals),
  - Secret contents **never** logged (forced Metadata for secrets regardless of policy),
  - `Request` for writes to RBAC, policies, tenants, exec commands (record the command).
- Record format: JSON lines (stable schema): request ID, timestamp, user info (incl.
  impersonator), source IPs, user agent, verb, resource, name, namespace, response code,
  latency, decision explanations (which RBAC binding allowed; which admission policy
  denied).
- **Backends**: local file with rotation (default), plus a buffered async shipper to
  external sinks (webhook/OTLP) `[later]` with back-pressure policy: `drop-oldest` (default)
  or `block` (compliance mode – API slows rather than losing audit records).
- Tamper evidence `[later]`: hash chaining of audit records per server (each record
  includes hash of previous), periodic signed checkpoints.

### Agent-side audit
Exec/attach/port-forward/files sessions are audited at the API (they go through it). Node
debug sessions additionally logged on the node.

## Open questions
- Events: fan-out query vs async replication.
- Should audit records be queryable via API (`rwctl audit --user alice --since 1h`)? Useful
  for small clusters without a SIEM; `[later]`.

## Build checklist
- [ ] Event store with aggregation, rate limits, retention; Events API.
- [ ] Audit stage in request pipeline; policy evaluation; file backend.
- [ ] Forced redaction rules for secrets.
