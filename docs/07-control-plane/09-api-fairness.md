# API Priority, Fairness and Overload Protection

**K8s analogue:** API Priority and Fairness (APF: FlowSchemas, PriorityLevelConfigurations,
shuffle sharding, seats), `--max-requests-inflight`, client-side rate limiting
(client-go QPS/burst), watch cache, `429 Too Many Requests` with `Retry-After`.

## The problem

The API is shared by: humans (rwctl), agents (thousands), controllers (built-in and
custom), CI/CD pipelines, operators with bugs. One misbehaving client (a controller
LISTing all pods every second; a script in a loop) can starve everyone else, including
the agents' lease renewals – which causes nodes to be marked dead, which causes mass
rescheduling, which causes more load: a **metastable failure** spiral.

Kubernetes' APF (GA 1.29) addresses this with classification into priority levels, fair
queuing among flows within a level, and seat-based concurrency accounting.

## Rustwain design (APF-like, simplified)

### Classification
Each request → **(priority level, flow key)**:
- Priority levels (built-in, ordered):
  1. `exempt` – server-to-server, emergency admin (`system:masters` equivalent, rate-limited
     still by a large ceiling).
  2. `node-critical` – agent lease renewals and node status (tiny, must never starve).
  3. `system` – built-in controllers and scheduler.
  4. `workload-high` – custom controllers marked important.
  5. `default` – everything else authenticated.
  6. `background` – bulk/batch clients, CI.
  7. `anonymous` – (if enabled at all).
- Flow key: typically the user/service account identity (or identity + namespace).
- `FlowRule` objects (≈ FlowSchema) map subjects/verbs/resources to levels.

### Concurrency accounting
- Each level has a share of total concurrency **seats**.
- Request cost: 1 seat for simple get/update; **list/watch-init costs proportional to
  estimated objects returned** (estimated from index cardinality – we know exactly how
  many objects match because indexes are in memory!) – Kubernetes estimates via object
  counts too (`width` of requests).
- Within a level, **fair queuing across flows** (shuffle sharding: each flow hashed to a
  small subset of queues so one heavy flow can't occupy all queues).
- Borrowing: idle levels lend seats to busy ones (Kubernetes added borrowing in 1.26).

### Overload behaviour
- Queue full → `429` with `Retry-After` computed from queue drain rate.
- Client library implements **adaptive client-side throttling** (Google SRE "client-side
  throttling": track accept ratio, reject locally with probability when the server is
  rejecting) – prevents retry storms.
- Watches: long-running, counted separately (limit concurrent watches per identity);
  initial events of watch-list count seats; ongoing delivery is not seat-based but buffer
  bounded.

### Protecting the store
- Write rate limits per tenant (token bucket) in addition to concurrency: e.g. one tenant
  can't produce more than N writes/s, protecting the Raft log.
- Object count quotas per tenant (see [quotas](../08-workloads/07-quotas-limits-priority.md)).

### Protecting agents' critical path
- `node-critical` level has reserved seats that can't be borrowed by others.
- Leases are leader-local (not Raft writes), so even a saturated Raft log can't delay
  heartbeats – a structural defence against the spiral.
- **Eviction safeguard**: if the API server itself is overloaded (queueing high), the node
  lifecycle controller **suspends** marking nodes dead based on missed heartbeats (it
  can't trust that silence means death) – "control-plane self-awareness".

## Visibility
- `rwctl api fairness` shows per-level/per-flow queue lengths, rejections, top flows.
- Each `429` includes the matched rule and level in a header for debuggability.

## Open questions

- Default number of levels – fewer is simpler. Possibly collapse 4–6.
- Should seats be estimated or measured adaptively (actual CPU time per request type)?

## Build checklist

- [ ] Classifier + FlowRule kind.
- [ ] Seat-based concurrency limiter with shuffle-sharded fair queues and borrowing.
- [ ] Cost estimation from index cardinality.
- [ ] Client-side adaptive throttling in `rustwain-client`.
- [ ] Overload test: misbehaving client must not affect lease renewals.
