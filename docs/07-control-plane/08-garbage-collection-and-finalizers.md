# Garbage Collection and Finalizers

**K8s analogue:** ownerReferences, garbage collector controller (dependency graph),
deletion propagation (`Orphan`, `Background`, `Foreground`), finalizers,
`blockOwnerDeletion`, namespace deletion.

## Ownership graph

Objects point to owners via `metadata.owners`. Example:

```
Deployment web
 └─ Revision web-7d9f (≈ ReplicaSet)
     ├─ Pod web-7d9f-abc
     └─ Pod web-7d9f-def
StatefulSet db
 ├─ Pod db-0
 └─ Claim data-db-0   (optional ownership: retained by default)
```

When an owner is deleted, dependents are deleted according to the **propagation policy**:
- `Background` (default): delete owner immediately; GC deletes dependents afterwards.
- `Foreground`: owner gets `deletion` + `foregroundDeletion` finalizer; dependents deleted
  first (those with `blockOwnerDeletion`); then owner removed. Gives "fully gone" semantics.
- `Orphan`: remove owner refs from dependents; they live on.

## GC controller design

Kubernetes' GC builds a full in-memory graph of **every object in the cluster** (via
metadata informers on all resource types) – memory-heavy and slow to start in large
clusters (it must discover and watch every resource type, including CRDs; a broken
aggregated API can block GC discovery).

Rustwain:
- The store maintains an **owner index** (owner UID → dependent keys) as a secondary
  index (see [state store](02-state-store.md)). The GC controller doesn't need its own copy
  of the graph – it queries the index.
- On owner deletion events: look up dependents via index → act per policy.
- Orphan detection (dependents whose owner UID no longer exists – e.g. owner deleted while
  GC was down): periodic sweep of the owner index for dangling UIDs – O(index), not
  O(objects).
- Custom types participate automatically (the index is generic).

## Finalizers

A finalizer is a string on an object; deletion sets `deletion.requested_at` but the object
remains until all finalizers are removed by their controllers. Used for external cleanup
(delete cloud LB before Service disappears; detach volume before Disk deleted).

### The stuck-finalizer problem
In Kubernetes, objects stuck in `Terminating` forever are among the most common
operational complaints: the controller responsible for a finalizer was uninstalled, is
crashing, or is waiting on something external. Users resort to `kubectl patch ... -p
'{"metadata":{"finalizers":null}}'`, which can leak external resources.

Rustwain improvements:
1. **Finalizers are registered**: a controller declares the finalizers it owns
   (`FinalizerRegistration{name, controller, kinds}`). Adding an unregistered finalizer is
   rejected (or warned) – so a finalizer always has a known responsible party.
2. **Finalizer status**: controllers report progress per finalizer on the object's status
   (`finalizing: [{name, state: WaitingFor{external LB deletion}, since, attempts, last_error}]`).
3. **`rwctl why <object>`**: shows exactly what blocks deletion – which finalizers, which
   controller owns them, whether that controller is running (lease held?), its last error,
   and dependents blocking foreground deletion.
4. **Orphaned finalizer detection**: if the owning controller hasn't held its lease for a
   configured time, the object gets a condition `FinalizerControllerAbsent`; admins can
   `rwctl finalize --force <obj> --finalizer <name> --reason "..."` (audited, explicit).

## Namespace deletion
- Namespace deletion deletes every namespaced object in it (via the namespace index –
  single prefix scan, no discovery of resource types needed).
- Objects with finalizers keep the namespace `Terminating`; `rwctl why namespace/x` lists
  them.
- Kubernetes' namespace deletion can block on unavailable aggregated APIs (it can't
  enumerate their resources). Our aggregated APIs (if any) must register their namespaced
  kinds and handle deletion by namespace or explicitly opt out.

## TTL-based cleanup
- Finished Jobs: `ttlAfterFinished` (Kubernetes' TTL controller).
- Completed bare pods: per-tenant default TTL `[proposed]`.
- Events: their own store with time-based expiry.

## Open questions

- Should Claims created from StatefulSet templates be owned (deleted with the set)?
  Kubernetes added `persistentVolumeClaimRetentionPolicy` (WhenDeleted/WhenScaled:
  Retain|Delete). Keep that knob; default Retain.
- Hard reject vs warn for unregistered finalizers (compat with third-party controllers).

## Build checklist

- [ ] Owner index in store; GC controller using it.
- [ ] Propagation policies with tests (including foreground with nested chains).
- [ ] FinalizerRegistration kind + admission check.
- [ ] Finalizer progress status helper in controller runtime.
- [ ] `rwctl why` for deletion blockers.
