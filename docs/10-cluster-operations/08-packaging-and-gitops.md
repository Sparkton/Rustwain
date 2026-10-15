# Packaging, Templating and GitOps

**K8s analogue:** Helm (charts, Go templates, releases stored as Secrets, hooks), Kustomize
(overlays, patches, generators), Carvel, Jsonnet/CUE, ApplySet (KEP-3659,
`kubectl apply --prune`), Argo CD, Flux.

## The problem space
1. **Templating/configuration**: one app, many environments (dev/stage/prod differences).
2. **Packaging/distribution**: share an app definition (with dependencies) with others.
3. **Release lifecycle**: install, upgrade, rollback, uninstall as a unit; track what
   objects belong to a release; prune removed objects.
4. **GitOps**: Git is the source of truth; a controller continuously applies it and reports
   drift.

## Kubernetes ecosystem critique
- **Helm**: Go text/template over YAML is error-prone (indentation via `nindent`,
  stringly-typed values, no schema by default); release state stored in cluster Secrets;
  hooks are a parallel lifecycle system; CRD handling awkward. But it's the de-facto
  distribution format with a huge catalogue.
- **Kustomize**: template-free overlays and patches – cleaner, but limited (no loops/
  conditionals), and strategic-merge-patch semantics are Kubernetes-specific.
- **Pruning**: `kubectl apply --prune` was alpha for years and dangerous; ApplySet is the
  newer standardised approach.

## Rustwain approach

### 1. Apply sets in core `[proposed]`
An **ApplySet** is a first-class object: a named set of objects applied together, with
a recorded inventory (kind/namespace/name/uid of members). `rwctl apply --apply-set=web
-f dir/` applies all, then prunes members no longer present in the input. Server-side
`ApplySet` objects make release tracking native (no Secrets-encoded Helm release blobs) and
power `rwctl get applyset/web` (status: all members Ready?).

### 2. Native overlays (Kustomize-like, typed)
- `rustwain.overlay.yaml`: base references, patches as **apply configurations** (SSA
  semantics – same merge rules as the server, no special patch dialect), name
  prefixes/suffixes, label/annotation injection, config/secret generators with content
  hashes (→ automatic rollouts on change).
- Built into `rwctl` (`rwctl apply -k overlays/prod`).

### 3. Typed values for parameterisation
Rather than a template language over YAML, parameterisation = **overlays + a small set of
typed variables** substituted into well-defined fields (`${replicas}` only allowed where
the schema says the type matches) `[proposed]`. If more power is needed, users generate
manifests with real languages (CUE, Jsonnet, or Rust/any language programs) – we don't
embed a template engine.

### 4. Helm compatibility via import
`rwctl import helm <chart> --values v.yaml` renders the chart (requires executing Go
templates – we'd need a Go template engine in Rust: the `gtmpl` crate implements Go
templates partially; Sprig functions are a long tail) → translates Kubernetes objects →
Rustwain manifests. Practical alternative: users run `helm template` themselves and pipe
to `rwctl import`. Lean: **the pipe approach** – zero maintenance burden.

### 5. GitOps controller `[later]`
A built-in `GitSource` + `Sync` controller (Flux-like, minimal): fetch a Git repo/OCI
artifact at an interval or on webhook, verify commit signatures, apply via ApplySet,
report drift and health. Rust Git: `gix` (gitoxide, pure Rust). Kept small; Argo CD-class
UI features out of scope.

### 6. OCI artifacts for distribution
Package = an OCI artifact containing manifests + overlay definitions + schema of
variables; pushed to any OCI registry; signed with Sigstore. `rwctl apply oci://ghcr.io/acme/web:1.4`.

## Open questions
- Typed variable substitution: expressive enough? Might push users to external generators.
- How far to support Kubernetes-ecosystem formats (Helm charts are everywhere).

## Build checklist
- [ ] ApplySet kind + server-side inventory + prune.
- [ ] Overlay engine using SSA apply configurations.
- [ ] Generators with content hashing.
- [ ] OCI artifact packaging/push/apply.
- [ ] GitSource/Sync controller spike with gix.
