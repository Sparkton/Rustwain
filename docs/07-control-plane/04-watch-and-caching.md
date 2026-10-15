# Watch and Caching

**K8s analogue:** apiserver watch cache (cacher), etcd watch, client-go informers
(Reflector, DeltaFIFO, Indexer), ListWatch, bookmarks, resourceVersion semantics,
`WatchList` streaming lists (KEP-3157), consistent reads from cache (KEP-2340).

## Why watch is the heart of the system

Every controller and every agent is level-triggered on a local cache kept fresh by
watches. The cost of watch fan-out dominates control-plane resource usage at scale:
5,000 nodes each watching pods on their node + all services/endpoints = millions of
event deliveries per minute during a large rollout.

## Server-side: the watch hub

Because the store is in-memory MVCC (see [state store](02-state-store.md)), the store *is*
the cache. The watch hub sits on top:

```
Raft apply loop ──▶ change events (rev, key, old, new)
                      │
                      ▼
              ┌─────────────────┐
              │  watch hub      │  per (kind) event ring buffer (history window)
              │                 │  per-watcher filters: prefix / label / field index
              └──┬────┬────┬────┘
                 ▼    ▼    ▼
            watcher watcher watcher ... (bounded channel each)
```

Key techniques:
- **Encode once**: events are serialised once per wire format and shared (`Bytes`)
  among all matching watchers. Kubernetes added this (watch cache caching encoded
  objects) after finding encoding dominated CPU.
- **Index-based dispatch**: watchers registered by index key (e.g. `spec.node=node-7`) are
  found via hash lookup, not by evaluating every watcher's predicate for every event.
  (Kubernetes' cacher has "trigger" functions for this, for a few fields.)
- **Bounded per-watcher buffers**: a slow watcher whose buffer fills is **terminated** with
  a resumable error (client resumes from its last revision) rather than blocking the hub
  or growing memory unboundedly.
- **Bookmarks**: periodic `Bookmark{revision}` events so idle watchers advance their
  resume point (avoids `TooOld` on resume after quiet periods).
- **Progress notifications**: allow consistent reads to confirm cache freshness.

### API sketch: hub dispatch

```rust
pub struct WatcherId(u64);

/// One applied change, as produced by the state machine's apply loop.
pub struct Change {
    pub rev: Revision,
    pub kind: KindId,
    pub key: Key,
    pub prev: Option<Arc<StoredObject>>,   // None on create
    pub next: Option<Arc<StoredObject>>,   // None on delete
}

/// What a watcher asked for, compiled once at registration.
pub struct WatchFilter {
    pub kind: KindId,
    pub namespace: Option<NamespaceId>,
    /// At most one *routing* term – the most selective indexed predicate
    /// (a field index like spec.node, or one label equality). Used for dispatch.
    pub route: Route,
    /// Remaining predicates (other labels, set-based selectors), evaluated only on
    /// watchers the route already selected.
    pub residual: Option<CompiledSelector>,
    pub projection: Option<ProjectionId>,   // [proposed] field projection
    pub format: WireFormat,                 // Binary | Json
}

pub enum Route {
    Index { index: IndexId, value: IndexValue },  // spec.node=node-7, label app=web
    Namespace,                                     // all of `namespace`
    Kind,                                          // cluster-wide catch-all
}

/// Dispatch tables, per kind. Mutated only by the hub task (register/unregister),
/// so no locks on the hot path.
struct KindRoutes {
    by_index: HashMap<(IndexId, IndexValue), SmallVec<[WatcherId; 4]>>,
    by_namespace: HashMap<NamespaceId, SmallVec<[WatcherId; 4]>>,
    catch_all: Vec<WatcherId>,
    /// Index values to extract from each object to look up `by_index`, i.e. only the
    /// indexes some watcher is actually routed on.
    active_indexes: SmallVec<[IndexId; 8]>,
}

struct WatcherSlot {
    filter: WatchFilter,
    tx: BoundedSender<Arc<EncodedEvent>>,   // capacity e.g. 1,024 events
    last_sent: Revision,
}

/// Encoded lazily, at most once per (format, projection), shared by all recipients.
pub struct EncodedEvent {
    pub rev: Revision,
    pub kind: EventKind,                    // Added | Modified | Deleted | Bookmark | Error
    obj: Arc<StoredObject>,
    encodings: [OnceLock<Bytes>; 2],        // Binary, Json (projections: side table)
}

pub trait WatchHub: Send + Sync {
    fn register(&self, filter: WatchFilter, from: StartPoint)
        -> Result<WatchStream, WatchError>;      // TooOld if `from` < history window
    fn unregister(&self, id: WatcherId);
}
```

Dispatch for one `Change` – note it considers **both** the old and the new object,
because a watcher must see an object *leave* its filter:

```rust
fn dispatch(&mut self, ch: &Change) {
    let routes = &self.kinds[ch.kind];
    // 1. Candidates: union of watchers routed on old values and on new values.
    let mut cands = CandidateSet::new();                    // small, dedup by WatcherId
    for obj in [ch.prev.as_deref(), ch.next.as_deref()].into_iter().flatten() {
        for idx in &routes.active_indexes {
            for v in obj.index_values(*idx) {               // label/field values
                cands.extend(routes.by_index.get(&(*idx, v)));
            }
        }
        cands.extend(routes.by_namespace.get(&obj.namespace()));
    }
    cands.extend(&routes.catch_all);

    let mut event: Option<Arc<EncodedEvent>> = None;
    for id in cands {
        let w = &mut self.watchers[id];
        let was = ch.prev.as_ref().is_some_and(|o| w.filter.matches(o));
        let is  = ch.next.as_ref().is_some_and(|o| w.filter.matches(o));
        let kind = match (was, is) {
            (false, true)  => EventKind::Added,      // created, or label now matches
            (true,  true)  => EventKind::Modified,
            (true,  false) => EventKind::Deleted,    // deleted, or moved out of filter
            (false, false) => continue,
        };
        let ev = shared_event(&mut event, ch, kind); // one Arc per (change, kind)
        if w.tx.try_send(ev).is_err() {              // full: never block the hub
            self.terminate(id, WatchError::Overloaded { resume_from: w.last_sent });
            continue;
        }
        w.last_sent = ch.rev;
    }
}
```

Cost per change: O(active indexes × values on the object + matched watchers), *not*
O(watchers). With 5,000 agents each routed on `spec.node`, a pod update touches one
`by_index` bucket (two if the node changed, which for pods it can't) and wakes one or
two watchers. Encoding happens on the first `try_send` that needs the format, inside the
receiving connection's task (`OnceLock` makes it once-only), so the hub thread does no
serialisation at all.

Bookmarks: a timer per kind enqueues `Bookmark(rev)` to every watcher of the kind whose
`last_sent` is older than 60 s – a pass over the slot table once a minute, not per event.

## Watch protocol

```
WatchRequest {
  kind, namespace?, selector (labels, fields), from: Revision | Now | ListThenWatch,
  allow_bookmarks: bool, send_initial_events: bool,
  delta: bool,     // [proposed] receive field-level diffs instead of full objects
}
WatchEvent = Added(obj) | Modified(obj | diff) | Deleted(obj_tombstone) | Bookmark(rev)
           | InitialEventsEnd(rev) | Error(TooOld | Overloaded | ...)
```

### List-then-watch without the memory spike
Kubernetes' classic informer start: `LIST` (one huge response, the apiserver must
materialise and encode it all at once → memory spike; many controllers restarting
simultaneously after an apiserver restart can OOM it) then `WATCH` from the list's RV.
KEP-3157 introduced **WatchList**: stream the initial state as watch events
(`sendInitialEvents=true`) then a bookmark marking the end.

Rustwain: **WatchList is the only way** – no separate large LIST for informer bootstrap.
`List` exists for CLI-style paged queries only.

### Delta events `[proposed]`
For large objects that change small fields (e.g. node status, endpoint sets), send a
structural diff (JSON-merge-patch-like, typed) against the previous revision the client
has. Client keeps the previous version anyway (it's a cache). Saves bandwidth and decode
CPU; requires clients to track base revisions precisely (if missing → resync).

## Client side: the Rust controller runtime

Equivalent of client-go's informers (and Rust's `kube-rs` `reflector`/`watcher`):

```rust
let pods: Store<Pod> = reflector(client.pods_all(), WatchParams::default()
        .fields("spec.node", &my_node)).run();   // keeps local cache via watch-list
let idx = pods.index_by(|p| p.spec.service_account.clone());
```

- **Shared informers** per process: many controllers sharing one watch per kind (as in
  client-go's `SharedInformerFactory`).
- Cache stores `Arc<Object>`; events carry `Arc` – zero-copy sharing among handlers.
- **Memory**: cache only what's needed; projection support (`fields: ["metadata",
  "spec.node"]`) `[proposed]` – server-side field projection for watches so a controller
  that needs only metadata doesn't cache full objects (Kubernetes has metadata-only
  informers via `PartialObjectMetadata`; generalise it).
- Resync: periodically re-enqueue all keys (level-triggered safety net), default long
  (10 min), jittered.
- Resume after disconnect from last seen revision; on `TooOld`, re-watch-list and compute a
  diff against the local cache to emit synthetic events (so handlers see a consistent
  stream).

## Consistency guarantees to document

1. Events for a single key are delivered in revision order.
2. Events across keys within a watch are in revision order (global order) – Kubernetes
   guarantees this per watch too.
3. A watch from revision R delivers every change with revision > R (unless TooOld).
4. Cache reads after a write by the same client: **read-your-writes** isn't automatic with
   caches – client APIs expose `wait_for_revision(r)` so a controller can ensure its cache
   has caught up to its own write before acting again (avoids the classic "controller acts
   twice on stale cache" bug, e.g. creating duplicate pods; Kubernetes uses expectations
   (`ControllerExpectations`) to work around this).

## Scale targets

| Scenario | Target |
|---|---|
| Watchers | 50k concurrent watches per server |
| Event fan-out | 1M event deliveries/s per server (pre-encoded) |
| Initial sync of an agent (its 100 pods + services/endpoints of cluster) | < 2 s at 10k services |
| Server restart → all clients resynced | no memory spike > 1.5× steady state |

## Open questions

- Delta events: worth the complexity? Measure endpoint/node-status heavy workloads.
- Field projection for watches: requires per-projection encoding caches.

## Build checklist

- [ ] Watch hub with index dispatch, encode-once, bounded buffers, bookmarks.
- [ ] Watch-list initial events.
- [ ] `rustwain-client` reflector/store/index library with shared informers.
- [ ] Simulated scale test: 5k fake agents watching (in one process, many connections).
