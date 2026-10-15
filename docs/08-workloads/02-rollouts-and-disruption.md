# Rollouts and Disruption

**K8s analogue:** Deployment RollingUpdate/Recreate, `progressDeadlineSeconds`,
`kubectl rollout`, PodDisruptionBudget, Eviction API, `kubectl drain`, Argo Rollouts /
Flagger (canary, blue-green, analysis).

## Rolling updates (baseline)
- `maxSurge` (extra pods above desired during update) and `maxUnavailable` (pods allowed
  below desired).
- New pods must be **Ready** for `minReadySeconds` before counting as available.
- If no progress within `progressDeadline` → condition `Progressing=False,
  reason=ProgressDeadlineExceeded`. Kubernetes stops there (does **not** roll back
  automatically) – surprising to many. Rustwain: configurable
  `onFailure: Pause | Rollback` (default `Pause`, with clear status).

## Canary and blue-green in core `[proposed]`

Progressive delivery is the norm; Kubernetes needs Argo Rollouts/Flagger with their own
CRDs that replace Deployments. Since Rustwain owns the dataplane and the workload
controller, we can do it natively:

```yaml
kind: Deployment
spec:
  strategy:
    canary:
      steps:
        - weight: 5            # 5% of traffic to new revision (service-level weighting)
        - pause: 10m
        - analysis: { ref: error-rate-check }
        - weight: 25
        - pause: 10m
        - weight: 100
      traffic: Service          # Service | Gateway (HttpRoute weights) | Replicas (pod-count based)
```

- **Traffic weighting**: endpoint weights in EndpointSets (socket-LB backend selection is
  weighted) → precise percentages independent of replica counts. Kubernetes can only
  approximate canaries by replica ratio unless a mesh/ingress is involved.
- **Analysis**: hooks evaluating a condition – initially via an external `AnalysisProvider`
  plugin (metrics are deferred), or a Job-based analysis (run a test Job; success
  proceeds).
- **Blue-green**: `strategy.blueGreen: { previewService, autoPromote, scaleDownDelay }`.
- `rwctl rollout promote/abort/pause/resume/undo`.

## Disruption budgets

`DisruptionBudget { selector, minAvailable | maxUnavailable, unhealthyPodEvictionPolicy }`:
- Voluntary disruptions (drains, rebalancing, preemption best-effort, autoscaler scale-down)
  must go through the **Eviction** subresource, which checks the budget atomically
  (compare with disruption controller's computed `allowedDisruptions` and decrement in
  one store transaction – Kubernetes does this with the PDB status `disruptionsAllowed` +
  resourceVersion check).
- Involuntary disruptions (node crash) aren't prevented but are counted.
- `unhealthyPodEvictionPolicy: AlwaysAllow` default `[proposed]` – Kubernetes' original
  default (`IfHealthyBudget`) made drains get stuck on already-broken pods; 1.26+ added
  this option. Make the sane behaviour default.

### Drain
`rwctl node drain <n>`:
1. Cordon (unschedulable taint).
2. Evict pods via Eviction API, respecting budgets; retry on 429 with backoff.
3. DaemonSet pods are left (they tolerate unschedulable) unless `--include-daemons`.
4. VMs with `LiveMigrate` → migration instead of eviction.
5. Report progress and blockers ("waiting on budget `db-pdb`: 0 disruptions allowed
   because db-2 is unready").
Kubernetes drain is client-side logic in kubectl (if the client dies, drain stops).
Rustwain: **server-side `Drain` object** `[proposed]` handled by a controller – resumable,
observable, usable by automation (upgrades, autoscaler).

### Node maintenance windows
`MaintenancePolicy` `[later]`: when automated drains (e.g. OS upgrades) may run, how many
nodes concurrently per zone, respecting budgets.

## Open questions

- Analysis without metrics: rely on plugins initially – acceptable?
- Weighted endpoints interplay with topology-aware routing (weights within zone?).

## Build checklist

- [ ] RollingUpdate with `onFailure` policy.
- [ ] Canary steps with weighted endpoints; blue-green.
- [ ] Disruption controller + atomic Eviction subresource.
- [ ] Server-side Drain object and controller.
