# Extensibility: Custom Types, Operators, Aggregation

**K8s analogue:** CustomResourceDefinitions (CRDs: schemas, versions, conversion
webhooks, subresources, printer columns, validation with CEL), API aggregation
(APIService), operators (Operator SDK, controller-runtime, kube-rs), Operator Lifecycle
Manager.

## Why extensibility is central
Kubernetes' biggest strength is that anyone can add a resource type and a controller, and
it feels native (`kubectl get certificates`). Thousands of projects depend on this. A
Kubernetes alternative without first-class extensibility is a toy.

## Custom types

```yaml
kind: CustomType
metadata: { name: databases.acme.io }
spec:
  group: acme.io
  names: { kind: Database, plural: databases, short: [db] }
  scope: Namespaced
  versions:
    - name: v1
      served: true
      storage: true
      schema: { ... OpenAPI v3 subset with rustwain extensions ... }
      subresources: { status: {}, scale: { specReplicasPath: .spec.replicas, ... } }
      columns: [ { name: Ready, path: .status.conditions[?(@.type=="Ready")].status } ]
      validations: [ { expr: "self.spec.replicas <= 9", message: "max 9" } ]   # CEL
  conversion: { strategy: Expressions | Webhook | None }
```

### Storage encoding
Kubernetes stores CRs as JSON in etcd and decodes into unstructured maps – slow and
memory-heavy compared with built-in protobuf types (CR-heavy clusters, e.g. with
thousands of Argo/Crossplane objects, see apiserver memory balloon).

Rustwain `[proposed]`: compile each custom type's schema into a **schema-driven binary
encoding** (field numbers assigned from the schema, like protobuf; or store as
`postcard`-encoded dynamic values with an interned key table). Watchers get the same
encode-once fan-out as built-ins. Unknown fields: pruned (structural schemas required –
Kubernetes made this mandatory in `apiextensions/v1`).

### Versions and conversion
- Exactly one **storage version**; others served via conversion.
- Conversion strategies:
  - `None` (only schema-identical versions),
  - `Expressions` `[proposed]` – declarative field mappings/CEL expressions evaluated
    in-process (no webhook needed for common renames/restructurings),
  - `Webhook` – call out (with the same guardrails as admission webhooks).
- **Storage version migration** built-in: when the storage version changes, a controller
  rewrites all stored objects to the new version (Kubernetes needs the separate
  `kube-storage-version-migrator`; forgetting it bites people at CRD version removal).

### Same machinery as built-ins
Custom types get: watch/list/indexes (label + declared field indexes!), SSA, GC/owners,
finalizer registrations, quotas (`count/databases.acme.io`), RBAC, admission policies,
printer columns, `rwctl explain`. Custom types should be indistinguishable from
built-ins to clients.

## Operators / custom controllers

### Rust SDK
The `rustwain-controller-runtime` crate (see [controllers](05-controllers.md)) is the
public SDK:
- Typed client generation from a CustomType (`rwctl codegen rust databases.acme.io` or a
  derive macro defining the type in Rust and **generating the CustomType manifest** from it
  – the kube-rs `#[derive(CustomResource)]` approach, which is very ergonomic).
- Reflectors, reconcilers, leases with fencing, finalizer helpers, status/conditions
  helpers, test harness with simulated API.

### Other languages
- gRPC API + generated protobuf makes clients in other languages possible (Go, Python),
  but the project only maintains the Rust SDK.

### Running operators
Operators are just workloads (Deployments with service accounts and RBAC). Packaging:
see [packaging and GitOps](../10-cluster-operations/08-packaging-and-gitops.md). No OLM
equivalent initially.

## API aggregation `[later]`
For APIs that can't be stored in the store (computed resources like metrics, or
proxies to external systems), register an `ApiService` that the API server proxies for a
group/version. Kubernetes uses it for metrics-server and others.
Guardrails: the aggregated server's unavailability must not break discovery or namespace
deletion (known Kubernetes problem) – discovery marks the group unavailable but continues.

## In-process extensions? (WASM is out of scope)
Some systems allow loading plugins into the API server process (WASM admission, etc.).
Project constraints exclude WASM. Extensions run out-of-process (controllers, webhooks)
or as declarative expressions (CEL). This keeps the core small and safe.

## Open questions

- Dynamic binary encoding design for custom types – protobuf-like with field numbers vs
  self-describing compact format.
- Expression-based conversion expressiveness limits.

## Build checklist

- [ ] CustomType kind; dynamic object representation; structural schema validation.
- [ ] Schema-driven binary encoding for custom types.
- [ ] Expression conversion + storage version migration controller.
- [ ] Derive macro: Rust struct → CustomType manifest + typed client.
- [ ] Example operator in-repo (e.g. a toy `Database` operator) as SDK dogfooding.
