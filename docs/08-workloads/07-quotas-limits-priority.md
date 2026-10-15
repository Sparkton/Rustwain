# Quotas, Limits and Priority

**K8s analogue:** ResourceQuota (compute, storage, object counts, scopes), LimitRange
(defaults, min/max, ratio), PriorityClass (value, preemptionPolicy, globalDefault),
system-cluster-critical / system-node-critical, preemption.

## Resource quotas

Per namespace (Kubernetes) – Rustwain supports **per tenant and per namespace**
(hierarchical: namespace quotas must fit within tenant quota).

```yaml
kind: Quota
metadata: { name: compute, namespace: shop }
spec:
  hard:
    requests.cpu: "40"
    requests.memory: 160Gi
    limits.memory: 200Gi
    devices/gpu.nvidia: 4
    storage.requests: 2Ti
    storage/fast.claims: 10
    count/pods: 500
    count/services.loadbalancer: 2
  scopes: [ { priority: [high] } ]     # optional: only count pods in these priorities
status:
  used: { ... }
```

### Enforcement correctness
Kubernetes enforces quota in admission by reading quota status and doing a CAS on the
quota object's usage – under high create rates, quota objects become contention hotspots
(conflicts and retries).

Rustwain:
- Quota usage tracked **in the store's state machine** as part of the same Raft
  transaction that creates the object (atomic increment with check) `[proposed]` – no
  separate CAS, no drift. The admission layer pre-checks (fast reject), the state machine
  enforces (authoritative).
- A reconciler periodically recomputes usage from indexes to fix any drift from bugs.

## Limit ranges (defaults and bounds)

```yaml
kind: ResourceDefaults
metadata: { namespace: shop }
spec:
  container:
    default:  { requests: { cpu: 100m, memory: 128Mi }, limits: { memory: 256Mi } }
    min:      { memory: 16Mi }
    max:      { cpu: "8", memory: 32Gi }
    maxLimitRequestRatio: { memory: 2 }
  claim: { min: 1Gi, max: 1Ti }
```
Applied in admission (mutation then validation). Defaults are written into the object
(visible) – no hidden magic.

## Priority

```yaml
kind: PriorityClass
metadata: { name: high }
spec:
  value: 100000
  preemption: PreemptLowerPriority     # or Never
  description: "customer-facing services"
```

- Built-ins: `node-critical` (2,000,001,000), `cluster-critical` (2,000,000,000),
  `normal` (0, global default), `batch-low` (−1000) `[proposed]`.
- System priorities restricted to `rustwain-system` namespace (admission).
- Priority affects: scheduling queue order, preemption, eviction ranking under pressure,
  graceful node shutdown ordering, API fairness? (no – API fairness is by client identity).

## Preemption semantics
See [scheduler](../07-control-plane/06-scheduler.md). Notes:
- Preemption across tenants: governed by tenant policy (a tenant's high priority shouldn't
  let it evict another tenant's pods unless the cluster admin allows) `[proposed]`:
  priority classes are tenant-scoped or cluster-scoped; cross-tenant preemption only for
  cluster-scoped classes.

## Object count limits
- `count/<kind>` quotas protect the store from object floods (e.g. a buggy controller
  creating 100k ConfigMaps).
- Global default per-namespace object limits `[proposed]` (e.g. 10k of any kind) unless
  raised – Kubernetes has none by default, and etcd fills up.

## Open questions

- Quota in the state machine requires the state machine to understand resource
  arithmetic for quota'd kinds – a deliberate exception to "store knows nothing about
  workloads". Alternative: serialize quota updates via a single-threaded quota service on
  the leader. Evaluate both.

## Build checklist

- [ ] Quota kind (tenant + namespace hierarchy), scopes.
- [ ] Atomic quota enforcement path + drift reconciler.
- [ ] ResourceDefaults kind.
- [ ] PriorityClass kind and admission rules.
- [ ] Load test: 1,000 pod creates/s in one namespace with quota – no conflict storms.
