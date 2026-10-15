# Batch and Gang Scheduling

**K8s analogue:** Job (Indexed), JobSet, Kueue (quotas, queues, admission), Volcano
(gang scheduling, queues), scheduler-plugins coscheduling (PodGroup), MPI Operator,
Kubeflow training operators, Slurm-on-Kubernetes efforts.

## Why it's needed
AI/ML training, HPC and data processing jobs need:
- **All-or-nothing placement** (gang): a 64-worker training job is useless with 63
  workers; partial placement wastes expensive GPUs and can **deadlock** two jobs that each
  hold half the cluster.
- **Queues and fair sharing** between teams with quotas, borrowing, and preemption.
- **Topology-aware placement**: workers in the same rack/NVLink domain/network block.
- **Job-level admission**: don't create pods until the job can run (avoid pending-pod
  floods).

Kubernetes core has none of this; Kueue and Volcano add it with their own controllers and
(for Volcano) their own scheduler.

## Rustwain design

### Queues and quotas
```yaml
kind: Queue
metadata: { name: ml-team-a }
spec:
  parent: ml               # hierarchical queues (cohorts)
  quota:  { gpu: 32, cpu: 512, memory: 4Ti }   # nominal
  borrowing: { limit: { gpu: 32 } }             # may borrow idle quota from siblings
  lending:   { limit: { gpu: 16 } }
  fairSharing: { weight: 2 }
  preemption: { withinQueue: LowerPriority, reclaimBorrowed: Any }
```

### Workload admission
- Jobs (or any workload type) declare `queue: ml-team-a` and their total requirements.
- The **admission controller for queues** (Kueue's model) decides when a job may start:
  quota available (nominal + borrowable) → admit → the job's pods are created (or
  unsuspended); otherwise the job stays queued (no pods exist → no scheduler load).
- Ordering within a queue: priority then FIFO (or fair-share across users).

### Gang scheduling
- `PodGroup { minMember: 64, timeout }` implicitly created for gang jobs.
- Scheduler: pods of a group are **reserved** but not bound until `minMember` are
  reserved (Permit stage waits); if the timeout expires, all reservations are released
  and the group backs off. Prevents partial placement.
- Better: **group-at-once scheduling** `[proposed]` – the scheduler attempts to place the
  whole group in one cycle (batch scheduling of equivalent pods), with topology
  constraints, and either commits all bindings or none. Avoids holding reservations while
  waiting.

### Topology-aware placement
- Node attributes describe hierarchy: `zone > block > rack > host` (+ GPU NVLink domains).
- Group constraint: `topology: { require: rack }` or `prefer: block` with fallback.
- Scheduler finds a domain with enough free capacity for the whole group (bin-packing
  per domain) – uses domain-level capacity aggregates.

### Preemption for batch
- Reclaim borrowed quota by preempting lower-priority borrowers' jobs (whole-job
  preemption, not single pods – killing one worker kills the job anyway).
- Checkpoint-aware: jobs can declare `preemptionGrace` to checkpoint before termination.

### Multi-pod job types
- **JobSet**-like `JobGroup`: multiple replicated jobs (driver + workers + parameter
  servers) with shared lifecycle, DNS for peers (headless service auto-created), failure
  policies (restart the whole set on any worker failure).
- MPI: launcher + workers with SSH-less bootstrap via known hostnames.

## Interaction with services and autoscaling
- Queues inform cluster autoscaling: admitted-but-unschedulable jobs (quota OK, no nodes)
  trigger provisioning of the right node types (Kueue's "provisioning request" pattern).

## Open questions

- Should queues be cluster-scoped objects tied to tenants? (Yes – tenants map to queue
  cohorts naturally.)
- Fair-sharing algorithm: DRF (Dominant Resource Fairness) is standard for multi-resource
  fairness – use it.

## Build checklist

- [ ] Queue kind (hierarchical), quota accounting, borrowing/lending.
- [ ] Workload admission controller (suspend/unsuspend semantics on Jobs).
- [ ] Scheduler group-at-once placement with topology constraints.
- [ ] Whole-job preemption.
- [ ] JobGroup kind.
- [ ] Simulation: two gang jobs competing for half a cluster must not deadlock.
