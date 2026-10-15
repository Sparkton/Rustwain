# Admission and Policy

**K8s analogue:** admission controllers (compiled-in plugins like NamespaceLifecycle,
LimitRanger, ServiceAccount, ResourceQuota, PodSecurity, DefaultStorageClass…),
MutatingAdmissionWebhook / ValidatingAdmissionWebhook, ValidatingAdmissionPolicy (CEL,
GA 1.30), MutatingAdmissionPolicy (CEL, beta), OPA Gatekeeper, Kyverno.

## Why admission matters
Admission is where organisations encode rules: "no `:latest` tags", "all pods must have
resource requests", "images only from our registry", "add these labels", "inject this
sidecar". It's also where many outages come from.

## The webhook problem
Kubernetes admission webhooks are HTTP callouts from the API server to (usually)
in-cluster services:
- **Availability coupling**: if the webhook's pods are down and `failurePolicy: Fail`, the
  API rejects matching writes – potentially including the writes needed to fix the webhook
  (classic deadlock: webhook selects pods in all namespaces including its own; its pods
  can't be recreated). With `failurePolicy: Ignore`, policy silently doesn't apply.
- **Latency**: every matching write pays a network round trip (or several – chains of
  mutating webhooks, re-invoked if they mutate each other's fields).
- **Security**: ingress-nginx's validating webhook CVE (2025) showed a webhook can be an
  unauthenticated RCE vector.
- Gatekeeper/Kyverno run as large deployments with their own caches of the cluster.

Kubernetes' answer: in-process **CEL** policies (ValidatingAdmissionPolicy, and
MutatingAdmissionPolicy). That's the right direction.

## Rustwain admission chain

```
1. Built-in mutation (compiled Rust):  defaults, service account, tenant labels,
                                       image digest pin hint, isolation class default
2. Policy mutation (expression-based): user-defined MutationPolicy objects
3. Schema validation (generated)
4. Built-in validation (compiled Rust): semantic rules per kind
5. Built-in admission (compiled Rust):  quota, security level, limit ranges,
                                       namespace lifecycle, reference warnings
6. Policy validation (expression-based): user-defined ValidationPolicy objects
7. Webhooks (optional, discouraged):   with strict guardrails
```

### Expression language
Options: CEL (Google's Common Expression Language – Kubernetes' choice, has a Rust
implementation `cel-interpreter`), Rego (OPA – Rust implementation `regorus` by
Microsoft), or a custom DSL.

Decision `[proposed]`: **CEL**. Same language as Kubernetes VAP (familiar, portable
policies), non-Turing-complete with cost estimation (bounded evaluation time), good fit
for "expression over an object". Evaluate the Rust interpreter's completeness and
performance; we may need to contribute or fork. Compile expressions once at policy load;
evaluate against a typed object view (avoid converting Rust structs to generic JSON
per evaluation – implement CEL's value traits directly over our types `[open]`).

```yaml
kind: ValidationPolicy
metadata: { name: require-requests }
spec:
  match: { kinds: [Pod], tenants: ["*"], exclude: { namespaces: [rustwain-system] } }
  rules:
    - expr: "object.spec.containers.all(c, has(c.resources.requests.cpu) && has(c.resources.requests.memory))"
      message: "every container must set cpu and memory requests"
  action: Deny        # Deny | Warn | Audit
  params: { ... }     # optional parameter object reference
```

```yaml
kind: MutationPolicy
metadata: { name: add-team-label }
spec:
  match: { kinds: [Deployment] }
  mutations:
    - applyConfiguration: |
        Object{ metadata: Object.metadata{ labels: {"team": namespaceObject.metadata.labels["team"]} } }
```

Mutations are expressed as **apply configurations** (SSA semantics) so multiple mutation
policies compose predictably (field ownership resolves conflicts) and re-invocation loops
are unnecessary.

### Policy safety features
- `action: Warn` and `Audit` modes for rollout.
- **Dry-run evaluation**: `rwctl policy test policy.yaml --against cluster` evaluates a
  policy against existing objects and reports what would fail – before enabling.
- **Policy exemptions** with expiry (break-glass).
- Policies cannot match objects in `rustwain-system` unless explicitly scoped there –
  prevents bricking the cluster.
- Cost limits per expression and per request.

### Webhooks (still supported)
For logic that can't be expressed in CEL (e.g. call an external inventory system):
- Mandatory `timeout ≤ 3 s`, mandatory `failurePolicy` choice, and **automatic exclusion
  of the webhook's own namespace and of `rustwain-system`**.
- Circuit breaker: after consecutive failures, the API marks the webhook `Degraded`,
  applies its failure policy immediately without waiting for timeouts, and probes
  periodically – avoids every request waiting 3 s during an outage.
- mTLS to webhook with workload identity verification.
- Webhook status visible via `rwctl get admissionwebhooks` (latency, failure rate).

## Built-in admission plugins (initial)

| Plugin | Purpose |
|---|---|
| NamespaceLifecycle | No creates in terminating/nonexistent namespaces |
| TenantDefaults | Tenant label, isolation class minimum, default network policy linkage |
| ServiceAccountDefaults | Default SA; no token auto-mount unless requested |
| SecurityLevel | Pod security levels (see [workload security policy](../09-security/06-workload-security-policy.md)) |
| ResourceDefaults (LimitRange) | Defaults and min/max per container/pod |
| Quota | Enforce ResourceQuota atomically (see quotas doc) |
| PriorityResolution | Map priority class name → value; restrict system priorities |
| StorageClassDefault | Default class for claims |
| NodeRestriction | Agents can only modify their own node and pods bound to them |
| ImagePolicy | Registry allow-lists, signature verification requirement flag |
| ReferenceWarnings | Warn on missing referenced objects |
| DeprecatedFieldWarnings | Warn on deprecated API fields/versions |

## Open questions

- CEL implementation maturity in Rust; fallback plan if gaps.
- Allow Rego as an alternative engine? (Large ecosystem of Gatekeeper policies.) Possibly
  via `regorus`, as a plugin. Lean: no – one language.

## Build checklist

- [ ] Admission chain as tower layers with per-stage metrics hooks.
- [ ] CEL evaluation over typed objects; cost estimation.
- [ ] ValidationPolicy / MutationPolicy kinds; apply-configuration mutation.
- [ ] Policy dry-run against cluster state.
- [ ] Webhook client with guardrails and circuit breaker.
