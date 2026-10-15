# Object Model

**K8s analogue:** ObjectMeta, TypeMeta, spec/status split, labels/annotations,
ownerReferences, finalizers, managedFields and server-side apply, generation /
observedGeneration, conditions, namespaces.

## Anatomy of an object

```rust
pub struct Object<S, T> {
    pub type_meta: TypeMeta,         // group, version, kind
    pub metadata: ObjectMeta,
    pub spec: S,                     // desired state (user-owned)
    pub status: Option<T>,           // observed state (controller/agent-owned)
}

pub struct ObjectMeta {
    pub name: Name,                  // DNS-1123 subdomain/label validated newtype
    pub namespace: Option<NamespaceName>,
    pub uid: Uid,                    // assigned at create, never reused
    pub resource_version: Revision,  // store revision of last write
    pub generation: u64,             // bumped on spec change
    pub created_at: Timestamp,
    pub deletion: Option<Deletion>,  // { requested_at, grace_period }
    pub labels: Labels,              // indexed
    pub annotations: Annotations,    // not indexed, size-limited
    pub owners: Vec<OwnerRef>,       // GC graph
    pub finalizers: Vec<Finalizer>,
    pub managed_fields: FieldOwnership,
}
```

Kept from Kubernetes deliberately: spec/status split, labels/selectors, owners,
finalizers, generation. These are good ideas with a decade of validation.

## Changes from Kubernetes

### 1. Names and validation as types
Kubernetes validation is a big hand-written Go function per type; validation rules drift
between versions. Rustwain:
- Constraints expressed as newtypes and derive attributes:
  `#[validate(range(min = 1, max = 65535))] port: u16`, `NonEmpty<Vec<Container>>`.
- Cross-field rules in typed `validate()` impls.
- The same rules exported into OpenAPI (`x-rustwain-validations` with expression strings)
  so the CLI validates offline.

### 2. Compact field ownership
`managedFields` in Kubernetes stores, per manager, a JSON set of all owned field paths
(`f:spec: f:containers: k:{"name":"app"}: ...`). It's often **larger than the object
itself** and was so noisy that kubectl hides it by default.

Rustwain stores field ownership as:
- A per-object **field path dictionary** (interned paths) + per manager a bitset over the
  dictionary. Typically 10× smaller.
- Hidden from normal reads; available via `?show=ownership`.

### 3. Conditions standardised
Every status has `conditions: Vec<Condition>` with
`{type, status: True|False|Unknown, reason (CamelCase enum), message, observed_generation,
last_transition}`, and the **`Ready`** condition is mandatory for every kind that has a
controller. Lets generic tooling answer "is this thing done?" for any kind (`rwctl wait`).
Kubernetes recommended this pattern late (KEP-1623) and adoption is inconsistent.

### 4. Typed, size-limited annotations
Annotations are a dumping ground in Kubernetes (last-applied-configuration – up to the full
object size again!, controller state, feature flags for ingress controllers). Rustwain:
- Total annotation size limit (16 KiB default).
- No `last-applied-configuration` – server-side apply only.
- Well-known annotation keys are documented and typed; unknown keys allowed.

### 5. Tenant as a field
`metadata.tenant` (derived from namespace → tenant mapping, immutable) – makes tenant-level
quotas, policy and audit cheap (no lookup). See
[multi-tenancy](../09-security/05-multi-tenancy.md).

## Server-side apply (SSA)

The declarative merge semantic: "here is the set of fields I (manager X) care about and
their values".

Algorithm (same as Kubernetes):
1. Compute the fields present in the applied config.
2. Compare with current object's ownership: if another manager owns a field and the value
   differs → **conflict** (unless `force`, which transfers ownership).
3. Fields previously owned by X but absent now → removed (if no one else owns them).
4. Merge; record X's new owned set.

List semantics need annotations on the schema (`atomic`, `set`, `map` keyed by e.g.
`name`) – Kubernetes' `x-kubernetes-list-type`/`list-map-keys`. We express them as Rust
attributes on fields (`#[list(map, key = "name")]`).

Decision `[decided]`: **SSA is the primary write path** for humans and GitOps. `Update`
(full replace with CAS) for controllers that own whole objects. JSON merge patch for quick
edits. **No strategic merge patch** (Kubernetes-specific, complex, superseded by SSA).

## Object relationships

### Owner references (GC)
- `owners: [{kind, name, uid, controller: bool, block_owner_deletion: bool}]`.
- Exactly one `controller: true` owner at most (the managing controller).
- Cross-namespace owner refs not allowed (same as Kubernetes; cluster-scoped owners can
  own namespaced objects).
- See [garbage collection and finalizers](08-garbage-collection-and-finalizers.md).

### References to other objects
Fields like `pod.spec.volumes[].claim` refer by name within the namespace. Kubernetes has
no referential integrity (you can create a pod referencing a nonexistent secret; it just
stays pending). Rustwain:
- References are typed (`ObjectRef<Claim>`).
- Admission **warns** (not rejects – eventual consistency matters for apply ordering) when a
  referenced object doesn't exist; status shows `blocking: MissingReference{...}`.

## Namespaces and scoping
- Namespaced vs cluster-scoped kinds (same as Kubernetes).
- Namespace deletion: cascades to all contents; Kubernetes namespace deletion getting stuck
  `Terminating` (due to finalizers or unavailable aggregated APIs) is a notorious problem.
  Rustwain: `rwctl why namespace/x` lists exactly which objects/finalizers block deletion
  (see GC doc).

## Immutability
- Fields marked `#[immutable]` (e.g. `pod.spec.containers[].image`? – no, that's mutable in
  k8s; immutable ones: pod `volumes`, service `clusterIP`, claim `storageClass`, job
  `selector`) reject updates with a clear error listing the field.
- Whole-object immutability option for config/secret (`immutable: true`) – lets agents
  stop watching them (Kubernetes' immutable ConfigMaps/Secrets: big scalability win).

## Object size and count limits
- Per-object max 1 MiB.
- Per-namespace/tenant quotas on object counts (`count/pods`, etc.) – protect the store.
- Per-kind global soft limits with alerts.

## Open questions

- Should labels have typed values (numbers, booleans)? Kubernetes labels are strings with
  restricted charset; typed values enable range selectors (`gpu-mem > 40`). Lean: keep
  strings for labels; typed **node attributes** separately for scheduling.
- Referential integrity: warn vs reject?

## Build checklist

- [ ] `ObjectMeta` + derive macros for validation, list semantics, immutability.
- [ ] SSA engine with compact ownership encoding; port Kubernetes SSA test cases.
- [ ] Conditions helper library.
- [ ] Reference resolution + `MissingReference` blocking reasons.
