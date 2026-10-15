# Density and Packing

**K8s analogue:** max pods per node (110 default), bin-packing scoring
(`MostAllocated`, `RequestedToCapacityRatio`), overcommit via requests < limits, VPA
right-sizing, Karpenter consolidation, descheduler, Borg's resource reclamation.

## Why density matters
Most clusters run at 20–40% actual CPU utilisation while being "full" by requests.
Waste comes from: over-requested resources (users pad requests), fragmentation (no node
has enough left for the next pod), per-pod overhead, node overhead (system daemons), and
idle replicas.

## Levers

### 1. Low fixed overhead per node and per pod
- Agent + supervisors tiny (see budgets) → more allocatable.
- No pause containers, no sidecar proxies by default.
- MicroVM overhead minimised (snapshots, free page reporting, shared kernel pages).

### 2. Higher pods-per-node ceiling
Kubernetes' 110 default exists because of kubelet/PLEG scalability, IP block sizing
(/24 per node), and runtime overhead. Rustwain removes those structural limits:
event-driven agent (no relisting), dynamic IP blocks, tiny supervisors. Remaining limits
are real: PIDs, memory, conntrack (not used for service traffic), BPF map sizes, kernel
netns count (fine at thousands). Target: 500+ pods/node on large machines.

### 3. Better requests (right-sizing)
- Usage pipeline → **recommendations** in workload status (`recommendedRequests` from a
  decaying histogram, p95 + margin).
- `rwctl rightsizing report` – cluster-wide list of most over-provisioned workloads and
  potential savings.
- Optional automatic application via in-place resize (VPA-like, no restarts).

### 4. Overcommit with safety (Borg's lesson)
Borg reclaims resources: the scheduler uses *predicted usage* rather than limits for
low-priority work, and kills/throttles low-priority tasks if predictions are wrong.
Rustwain staged approach:
- Stage 1: requests = scheduling unit (Kubernetes model).
- Stage 2 `[later]`: **reclaimable capacity** – the agent reports `allocatable − actual
  usage of guaranteed/burstable (with margin)` as **reclaimable** resources; batch
  (BestEffort/`batch-low`) pods can be scheduled against reclaimable capacity, with
  `cpu.idle` and aggressive eviction when the primary workloads need it back. Equivalent to
  Kubernetes community projects (Koordinator's "batch resources", Crane).

### 5. Bin-packing vs spreading
- Default score: balanced (LeastAllocated weighted with spread across owners) – reliability.
- Pool-level policy: `packing: MostAllocated` for batch/cost-optimised pools – fewer nodes
  needed, autoscaler consolidation removes empties.
- Multi-resource fragmentation: score with dominant-resource awareness (avoid nodes where
  placing the pod leaves one resource exhausted and others stranded) –
  `RequestedToCapacityRatio`-like shape functions per resource.

### 6. Consolidation and rebalancing
- Autoscaler consolidation (see [autoscaling](../08-workloads/03-autoscaling.md)).
- Rebalancer evicts (budget-aware) to defragment.

### 7. Memory efficiency on node
- Proactive reclaim of idle page cache (`memory.reclaim`) for burstable pods.
- zswap for compressed swap of cold anonymous memory (Burstable pods with swap allowed).
- KSM for trusted microVMs (off for multi-tenant).
- composefs/overlay page cache sharing across pods using the same image layers (overlay
  already shares page cache for identical lower files; composefs extends this across
  images with identical files).

### 8. Idle workloads
- Scale-to-zero for HTTP services (activator) `[later]`.
- Suspended Jobs / CronJobs cost nothing.
- **Pod freezing** `[open]`: freeze idle pods (`cgroup.freeze`) and reclaim their memory
  to swap/zswap; thaw on first connection (activator). Like "serverless" warm-ish pools.

## Metrics for density (manual until observability lands)
- Requested vs allocatable vs used per node pool (`rwctl top pools`).
- Fragmentation index: fraction of free capacity unusable by the median pod shape.

## Open questions
- Reclaimable capacity model – Stage 2 timing.
- Pod freezing practicality (TCP keepalives, timers in frozen apps).

## Build checklist
- [ ] Recommendations in workload status.
- [ ] Pool packing policies in scheduler.
- [ ] Reclaimable capacity reporting (agent) `[later]`.
- [ ] Density benchmark: max pods/node at fixed memory with sample workloads.
