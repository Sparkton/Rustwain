# Scheduler

**K8s analogue:** kube-scheduler (scheduling framework: QueueSort, PreFilter, Filter,
PostFilter (preemption), PreScore, Score, Reserve, Permit, PreBind, Bind, PostBind),
scheduler profiles, extenders, descheduler, Kueue/Volcano for batch.

## Job

Assign each unscheduled pod to a node such that hard constraints hold and soft
preferences are optimised, fast, at scale, without double-booking.

## Kubernetes scheduler: how it works and where it hurts

- One pod at a time from a priority queue. For each pod: filter all (or a sample of)
  nodes → score feasible nodes → pick best → assume (update cache) → bind async.
- `percentageOfNodesToScore` (adaptive, ~50% at 100 nodes down to 5% at 5,000) trades
  quality for speed.
- Throughput: published targets ~100–300 pods/s for typical configs; inter-pod
  (anti-)affinity historically made it O(pods × nodes) and much slower.
- **No gang/batch awareness** (one pod at a time can deadlock gangs: half of a 64-GPU job
  placed, the other half can't fit, resources held forever).
- Topology (NUMA) unknown → kubelet admission failures after scheduling.
- Scheduler and kubelet have separate views of resources → races (kubelet rejects with
  `OutOfcpu`).

## Rustwain scheduler design

### Data structures
- **Cluster snapshot** in memory: nodes (allocatable, attributes, taints, topology,
  devices, free exclusive CPUs per NUMA, storage pools), pods (bound + assumed), indexed:
  - by node,
  - by label (for affinity) – **per topology domain counters** (`zone=a: {app=web: 3}`)
    so affinity/spread evaluation is O(domains), not O(pods).
- Updated incrementally from watches; scheduling cycles operate on an immutable
  generation (copy-on-write structures) so filtering can run in parallel threads without
  locks.

### Pipeline (framework, Rust traits)
```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
}
pub trait PreFilter: Plugin { fn pre_filter(&self, ctx: &mut CycleState, pod: &PodInfo) -> Status; }
pub trait Filter: Plugin   { fn filter(&self, ctx: &CycleState, pod: &PodInfo, node: &NodeInfo) -> Status; }
pub trait Score: Plugin    { fn score(&self, ctx: &CycleState, pod: &PodInfo, node: &NodeInfo) -> i64;
                             fn normalize(&self, scores: &mut [NodeScore]) {} }
pub trait Reserve: Plugin  { fn reserve(&self, ...) -> Status; fn unreserve(&self, ...); }
pub trait Permit: Plugin   { fn permit(&self, ...) -> PermitDecision; } // Allow | Wait(timeout) | Deny
pub trait PostFilter: Plugin { fn post_filter(&self, ...) -> PostFilterResult; } // preemption
```
Same extension points as Kubernetes (they're well designed). Plugins compiled in;
external logic via a gRPC **extender** plugin is possible but discouraged (latency).

### Built-in plugins
| Plugin | Type | Notes |
|---|---|---|
| NodeResourcesFit | Filter + Score | requests vs allocatable incl. pod overhead; scoring strategies: LeastAllocated (spread), MostAllocated (bin-pack), RequestedToCapacityRatio |
| NodeAttributes (selectors, affinity) | Filter + Score | typed attribute expressions |
| Taints/Tolerations | Filter + Score | |
| PodAffinity/AntiAffinity | PreFilter + Filter + Score | domain counters |
| TopologySpread | PreFilter + Filter + Score | domain counters; `minDomains`, `nodeTaintsPolicy` |
| VolumeTopology + Capacity | Filter + Reserve | WaitForFirstConsumer binding, local capacity reservation |
| Devices | Filter + Reserve | structured device allocation (see devices doc) |
| CpuTopology | Filter + Score | exclusive CPU / NUMA feasibility (prevents agent rejection) |
| IsolationClass | Filter | node supports class |
| ImageLocality | Score | prefer nodes with image layers present (compact per-node summary) |
| PortConflicts | Filter | host ports |
| Spread by owner (default) | Score | spread replicas of same owner across nodes/zones |
| Gang (Permit) | Permit | wait until all members reserved (see batch doc) |

### Throughput techniques
- **Parallel filtering and scoring** across nodes (rayon or a fixed thread pool) on the
  immutable snapshot.
- **Equivalence classes**: pods from the same template (same Deployment revision) have
  identical constraints → cache filter results per class per node generation; scheduling
  the 2nd–100th replica reuses most work. (Kubernetes had an "equivalence cache" and removed
  it due to correctness bugs with invalidation – we must design invalidation carefully:
  node generation counters + affinity domain counters as cache keys.)
- **Batch scheduling** `[proposed]`: pop up to N pods of the same class and place them in
  one cycle (assign greedily with incremental updates to the snapshot) – big win for
  rollouts and Jobs.
- **Node sampling** like `percentageOfNodesToScore` – configurable, adaptive.
- Target: **≥ 1,000 pods/s** for simple pods on 5,000 nodes; ≥ 200 pods/s with affinity.

### Binding
- Optimistic: reserve in local snapshot ("assumed"), write `binding` to API asynchronously.
- The binding write includes **allocations** (devices, exclusive CPUs, local-volume
  capacity, ports) – the agent receives a pod with all decisions made and only *verifies*.
- If binding fails (conflict, node gone), unreserve and requeue.

### Multiple schedulers / HA
- One active scheduler (leader-elected); standby on other servers.
- `schedulerName` field allows custom schedulers (same as Kubernetes) – a custom scheduler
  binary built on our framework crate.
- `[open]` Parallel optimistic schedulers (Omega-style shared-state scheduling with
  conflict detection at bind time via resourceVersion of node allocation records) –
  interesting for very large clusters; not initially.

### Queueing
- Priority queue ordered by pod priority, then creation time.
- **Unschedulable pods** park in a backoff queue and are re-tried **only when a relevant
  cluster event occurs** (node added, pod deleted on a node, etc. – Kubernetes'
  `QueueingHint` mechanism, 1.28+) instead of periodic blind retries.
- Each unschedulable pod gets a structured explanation:
  `Unschedulable{ nodes: 5000, reasons: {InsufficientCpu: 3200, TaintNotTolerated: 1500,
  AntiAffinity: 300} }` and the **closest node** with what's missing ("node-17 lacks
  250m CPU"). Kubernetes' "0/5000 nodes are available: ..." message is similar but terse.

### Preemption
- If a pod can't fit, find nodes where evicting lower-priority pods would make room;
  choose the node minimising (highest victim priority, number of victims, PDB violations).
- Respect disruption budgets where possible (Kubernetes: best-effort).
- Nominated node recorded; preemption victims get graceful termination; the preemptor
  waits.
- Preemption policy per priority class (`Never` for "don't preempt others").

## Descheduler / rebalancing `[later]`
The scheduler makes a decision once; clusters drift (new nodes empty, old nodes hot,
spread constraints violated after scale-down). A **rebalancer controller** periodically
proposes evictions (respecting budgets) to improve balance/packing – Kubernetes needs the
external `descheduler` project. Integrates with the autoscaler for consolidation (Karpenter
does this well).

## Open questions

- Equivalence caching correctness – needs extensive property testing.
- Batch scheduling interactions with fairness (one big Deployment monopolising cycles) –
  per-tenant fairness in the queue.
- Omega-style multi-scheduler.

## Build checklist

- [ ] `rustwain-scheduler` crate: snapshot, framework traits, plugins above.
- [ ] Domain-counter indexes for affinity/spread.
- [ ] Batch scheduling of equivalent pods.
- [ ] Event-driven requeue with queueing hints.
- [ ] Unschedulable explanations with closest-node analysis.
- [ ] Benchmark harness: kwok-like fake nodes (thousands of simulated nodes in one process)
      measuring pods/s and placement quality.
