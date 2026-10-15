# Controllers and Reconciliation

**K8s analogue:** kube-controller-manager (~40 controllers), controller-runtime, client-go
workqueues (rate limiting, dedup), leader election via Leases, expectations.

## The reconciliation pattern

```
watch events ─▶ enqueue key ─▶ worker pops key ─▶ reconcile(key):
                                                     read desired (spec) + observed (caches)
                                                     compute diff
                                                     act (create/update/delete via API)
                                                     update status (observedGeneration, conditions)
                                                     return Ok | Requeue(after) | Err (backoff)
```

Properties we require of every controller:
- **Idempotent**: reconcile(key) twice in a row = same result as once.
- **Level-triggered**: never depends on which event triggered it.
- **Bounded side effects per pass**: e.g. create at most N pods per reconcile (slow start –
  Kubernetes ReplicaSet creates pods in exponentially growing batches 1, 2, 4… to avoid
  flooding when creations fail, e.g. quota errors).
- **Uses expectations / revision waits** to avoid acting on stale caches.

## Controller runtime (Rust)

```rust
#[async_trait]
pub trait Reconciler: Send + Sync + 'static {
    type Object: Resource;
    async fn reconcile(&self, obj: Arc<Self::Object>, ctx: &Context) -> Result<Action, Error>;
    async fn cleanup(&self, obj: Arc<Self::Object>, ctx: &Context) -> Result<Action, Error> { Ok(Action::Done) }
    fn error_policy(&self, err: &Error) -> Action { Action::RequeueWithBackoff }
}

Controller::new(api.deployments_all())
    .owns(api.replicasets_all())       // events on owned objects map to owner key
    .watches(api.pods_all(), map_fn)   // custom mapping
    .concurrency(8)
    .run(reconciler);
```

This is close to `kube-rs`'s `kube::runtime::Controller` (which is excellent and a primary
reference). Built on:
- **Work queue**: dedup (a key queued once until processed), per-key serialisation (never
  two workers on the same key), rate limiting (per-item exponential backoff + overall
  token bucket), delayed requeues.
- **Shared reflectors** (see [watch and caching](04-watch-and-caching.md)).
- **Finalizer helper**: add finalizer on first reconcile, run `cleanup` on deletion,
  remove finalizer.
- **Status helper**: computes conditions, sets `observedGeneration`, writes via status
  subresource with SSA (field manager = controller name).

## Leader election and sharding

Kubernetes runs all controllers in one process with one leader-election lease; only the
leader runs anything (the others are hot standbys). Simple, but the leader does all work.

Rustwain:
- Controllers run in `rustwain server` processes; **per-controller leases** (store-native
  leases with fencing tokens = lease revision) so different controllers can lead on
  different servers → spreads load across servers.
- **Fencing**: every write from a controller includes its lease token as a precondition
  (`Txn` compare on lease key revision) so a deposed leader (GC pause, partition) cannot
  write after losing leadership. Kubernetes lacks this – a deposed controller-manager can
  briefly write concurrently with the new leader. (Rust has no GC pauses, but partitions
  still happen.)
- **Sharding** `[later]`: for very large clusters, shard a controller by namespace hash
  (each shard has its own lease) – needed for e.g. the endpoint controller at 100k+ pods.

## Built-in controllers (initial list)

| Controller | Watches | Does |
|---|---|---|
| namespace | namespaces | Cascading delete of contents, finalization |
| gc | all with owners | Orphan/foreground/background deletion |
| deployment | deployments, revisions, pods | Rollouts |
| statefulset | statefulsets, pods, claims | Ordered identity, per-replica claims |
| daemonset | daemonsets, nodes, pods | One pod per matching node |
| job / cronjob | jobs, pods | Completions, parallelism, schedules |
| disruption | budgets, pods | Compute allowed disruptions |
| node lifecycle | nodes, leases | Unreachable detection, taints, evictions |
| endpoint | services, pods | EndpointSets |
| identity | pods, namespaces | Security identities for policy |
| ipam | nodes, blocks | Pod CIDR blocks |
| service IP | services | VIP allocation repair |
| volume | claims, disks, classes | Provision/bind/expand/delete |
| attach | pods, disks, nodes | Attach/detach with fencing |
| snapshot | snapshots | Create/delete via drivers |
| quota | quotas, all counted objects | Usage tracking |
| autoscaler (HPA) | scalers, metrics | Replica count |
| image resolver | pods (unresolved refs) | Tag → digest |
| certificate | CSRs | Approve/sign node/workload certs |
| service account tokens | SAs | Token issuance (if not projected) |
| ttl | finished jobs/pods | Cleanup after TTL |
| tenant | tenants | Namespaces, default policies, quotas |
| lb (bare metal) | services(LoadBalancer), pools | VIP assignment, L2/BGP announcer selection |

## Testing controllers

- **Deterministic simulation**: in-memory API (store + watch hub) + simulated clock;
  controllers run against it; fake "agents" mark pods running. Property tests: "after
  quiescence, replicas == desired and all pods match template hash".
- **Fault injection**: random API errors (conflicts, timeouts), dropped watches, stale
  caches, restarts mid-reconcile.
- Assert **no thrashing**: total writes per reconcile cycle bounded after convergence.

## Open questions

- Per-controller leases spread across servers vs all-on-leader (simpler debugging).
- Expose the controller runtime as the public SDK for custom controllers (yes – see
  [extensibility](10-extensibility.md)).

## Build checklist

- [ ] `rustwain-controller-runtime` crate (queue, reflector integration, leases with fencing,
      finalizer/status helpers).
- [ ] Simulation harness + property tests per controller.
- [ ] Deployment controller as the first reference implementation.
