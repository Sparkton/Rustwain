# API Server

**K8s analogue:** kube-apiserver – REST API, discovery, OpenAPI, authentication,
authorization, admission, validation, conversion, storage, watch cache, subresources,
aggregation, priority and fairness.

## Responsibilities

1. Terminate client connections (TLS), authenticate.
2. Classify and rate-limit (fairness).
3. Authorize.
4. Decode, default, convert between versions.
5. Admission: mutate, validate (built-in + policy).
6. Persist via the store (with optimistic concurrency).
7. Serve reads (consistent or cached) and watches.
8. Subresources: `status`, `scale`, `exec`, `log`, `portforward`, `binding`, `eviction`, `files`.
9. Discovery and schema publication.
10. Audit.

## Transport and wire format

Decision `[proposed]`:
- **gRPC (HTTP/2) canonical API**, protobuf messages generated from Rust types.
- **JSON/HTTP gateway** on the same port (content negotiation): REST-ish paths
  (`/v1/namespaces/shop/pods/web-1`), JSON bodies, `?watch=true` as newline-delimited
  JSON stream. For curl, scripts, and humans.
- YAML only in the CLI (never on the wire).

### Generic resource service
Instead of one gRPC service per resource type (code explosion), a **generic resource
API** with typed payloads:

```protobuf
service Resources {
  rpc Get(GetRequest) returns (Object);
  rpc List(ListRequest) returns (ObjectList);
  rpc Watch(WatchRequest) returns (stream WatchEvent);
  rpc Create(CreateRequest) returns (Object);
  rpc Update(UpdateRequest) returns (Object);      // full replace with resourceVersion precondition
  rpc Apply(ApplyRequest) returns (Object);        // server-side apply
  rpc Patch(PatchRequest) returns (Object);        // JSON merge patch
  rpc Delete(DeleteRequest) returns (DeleteResponse);
  rpc DeleteCollection(...) returns (...);
}
message Object { TypeMeta type = 1; bytes payload = 2; /* encoded typed object */ }
```

Plus typed Rust client wrappers generated per kind (`client.pods("shop").get("web-1")`).
This mirrors how Kubernetes' REST is generic underneath, while keeping strong typing at
the edges.

### Encoding inside the store
- Objects stored in a compact binary encoding (protobuf or `postcard`/`bincode`-style
  with a version header) – not JSON. Kubernetes stores protobuf for built-ins but **JSON
  for CRDs** (large and slow to decode); we use the same binary encoding for custom types
  (schema-driven, see [extensibility](10-extensibility.md)).
- Encoding/decoding cost dominates API server CPU in Kubernetes at scale. Strategies:
  - Watch fan-out sends **pre-encoded bytes**: encode once per (object revision, wire
    format), share via `Bytes` (refcounted) across thousands of watchers.
  - Zero-copy decode where practical (`prost` + `bytes`).

## API groups, versions, kinds

- Paths: `/<group>/<version>/namespaces/<ns>/<resource>/<name>[/<subresource>]`.
- Core groups: `core` (pods, services, configs, secrets, namespaces, nodes, events),
  `apps` (deployments, statefulsets, daemonsets, rollouts), `batch`, `net`, `storage`,
  `policy`, `auth`, `devices`, `virt`.
- Versions: `v1alpha1`, `v1beta1`, `v1` with **conversion through an internal hub
  version** (Kubernetes' model: every external version converts to/from the internal
  representation). Conversion functions are Rust code (`From`/`TryFrom` impls), generated
  for trivial field mappings, handwritten for semantic changes; round-trip property tests
  required for every version pair (Kubernetes' fuzz round-trip tests are a good idea).

## Request pipeline

```
accept TLS conn ─▶ authn (cert / token / OIDC) ─▶ fairness classify & queue
  ─▶ authz (RBAC + node authorizer + policy) ─▶ decode + version convert to internal
  ─▶ defaulting ─▶ mutating admission (built-in, then policies, then webhooks)
  ─▶ validation (schema + built-in rules) ─▶ validating admission (policies, webhooks)
  ─▶ store txn (precondition on revision) ─▶ encode response ─▶ audit log
```

Every stage is a typed `tower::Layer`/middleware with metrics hooks (for when
observability lands). Timeouts per stage; total request deadline propagated.

## Reads: consistency choices
- **Linearizable read** (default for `Get` with no revision): served from the watch cache
  after confirming the cache is caught up to the leader's commit index (Raft **ReadIndex**:
  ask leader for current commit index; wait until local applied ≥ it). Kubernetes 1.31+
  does "consistent reads from cache" with etcd progress notifications – same idea.
- **Exact revision read** (`revision=N`): served from cache if still within the history
  window, else from store MVCC (until compacted).
- **Stale-ok read** (`consistency=any`): served from local cache immediately – for
  controllers' periodic resyncs and dashboards.
- Lists are always served from cache (the store is never scanned per list request) – a
  key Kubernetes scalability lesson: uncached LISTs of large collections hitting etcd
  can take down a cluster.

## Writes: concurrency
- Every object carries `resourceVersion` (= store revision of last modification).
- `Update` requires matching `resourceVersion` (compare-and-swap). Conflicts return
  `409 Conflict` with the current version so clients can retry.
- `Apply` resolves conflicts by field ownership (see [object model](03-object-model.md)).
- **Generation**: incremented on spec changes only; `status.observedGeneration` lets
  clients know when a controller has processed a change.

## Subresources
| Subresource | Purpose |
|---|---|
| `status` | Separate write path & permissions; spec changes ignored on status writes and vice versa |
| `scale` | Uniform scaling interface for any scalable kind (HPA uses it) |
| `binding` | Scheduler assigns pod to node (special permission) |
| `eviction` | Disruption-budget-aware delete (see rollouts doc) |
| `log`, `exec`, `attach`, `portforward`, `files` | Streaming via agent tunnel |
| `resize` | In-place resource changes for pods (separate permission) |
| `proxy` | `[skip]` – Kubernetes' API server proxy to pods/services is a security footgun |

## Discovery and schema
- `/discovery` lists groups/versions/resources with verbs, scope, short names – one
  aggregated document (Kubernetes added aggregated discovery in 1.27 to stop clients
  making hundreds of discovery requests).
- `/openapi/v3` generated from Rust types (`schemars`-style derive) – used by CLI for
  validation/completion.

## Large objects and limits
- Max object size: 1.5 MiB in Kubernetes (etcd request limit). Rustwain store design
  permits larger values but we cap at **1 MiB default** (configurable) – large objects
  hurt watch fan-out and memory. Big data belongs in volumes/object storage.
- Max list response pages; `limit`/`continue` pagination mandatory for lists > N items
  (server enforces a max page size so a naive client can't request 500k pods in one
  response).

## Horizontal scaling
All servers serve API requests (reads from local cache, writes forwarded to the Raft
leader). Clients load-balance across servers (client-side, from a server list) or via a
VIP/LB. Watch-only **learner replicas** can be added for large clusters.

## Kubernetes API compatibility gateway `[open]`
A separate optional component translating the Kubernetes REST API (for core kinds:
Pods, Services, Deployments, ConfigMaps, Secrets, Namespaces) to Rustwain's API, so that
`kubectl`, Helm and some operators could work. High value for adoption, but a
compatibility layer that is "mostly compatible" creates subtle bugs. Lean: import tool
first (`rwctl import`), gateway later if demand.

## Open questions

- Protobuf (prost) vs a Rust-native binary format for storage encoding; protobuf gives
  schema evolution discipline and polyglot clients.
- Generic resource service vs per-kind services in gRPC (generic chosen; validate
  ergonomics with the client generator).

## Build checklist

- [ ] `rustwain-api-types` crate: core kinds, metadata, versions, conversions.
- [ ] Code generator (build.rs or xtask): Rust types → protobuf, OpenAPI, typed clients.
- [ ] Request pipeline as tower layers; in-memory store backend for tests.
- [ ] ReadIndex-based consistent reads from cache.
- [ ] JSON gateway.
- [ ] Round-trip conversion property tests.
