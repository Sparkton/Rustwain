# Autoscaling

**K8s analogue:** HorizontalPodAutoscaler (v2: resource, pods, object, external metrics;
behavior policies), VerticalPodAutoscaler (recommender, updater, admission), Cluster
Autoscaler, Karpenter, KEDA (event-driven, scale to zero), metrics-server + Metrics API.

Note: observability is deferred, but autoscaling needs *some* metrics. We define a
minimal internal **resource usage pipeline** (CPU/memory/PSI per pod from agents) that is
not a general monitoring system – just what autoscaling and scheduling need.

## Resource usage pipeline (minimal)
- Agents compute per-pod usage (CPU rate from `cpu.stat`, memory working set from
  `memory.stat`, PSI) every 15 s and publish compact `PodUsage` summaries to an in-memory
  **usage store** on the servers (not the Raft store – ephemeral, last-N windows).
- Exposed via API (`rwctl top pods/nodes`) and consumed by autoscalers.
- Equivalent to metrics-server (which scrapes kubelet `/metrics/resource`), but push-based
  and built-in.

## Horizontal autoscaling (`Scaler`)

```yaml
kind: Scaler
spec:
  target: { kind: Deployment, name: web }
  min: 2
  max: 50
  metrics:
    - resource: { name: cpu, targetUtilization: 70 }      # % of request
    - pressure: { resource: cpu, targetSome: 10 }         # PSI-based  [proposed]
    - external: { provider: queue-depth, query: "orders", targetPerReplica: 100 }
  behavior:
    scaleUp: { stabilization: 0s, policies: [ { percent: 100, period: 15s }, { pods: 4, period: 15s } ] }
    scaleDown: { stabilization: 300s, policies: [ { percent: 10, period: 60s } ] }
  scaleToZero: { enabled: true, activator: true }        # [later]
```

- Algorithm: `desired = ceil(current × currentMetric / target)` per metric, max across
  metrics, with tolerance (10%), stabilization windows, rate policies – same proven HPA
  algorithm.
- **PSI-based scaling** `[proposed]`: CPU utilisation vs request is a weak signal (requests
  are often wrong); PSI `some` for CPU directly measures "tasks waiting for CPU" – scale
  when the service is actually contended.
- **External metrics** via a provider plugin interface (gRPC) – KEDA-like scalers (queues,
  streams, cron) without a separate project.
- **Scale to zero** `[later]`: needs an activator – the dataplane holds the first
  connection to a zero-replica service (socket-LB can redirect to an agent-local
  "activator" socket), triggers scale-up, then forwards. Like Knative's activator but
  integrated in the node dataplane.

## Vertical autoscaling `[later]`
- Recommender: histogram of usage (decaying) per container → recommended requests (target
  percentile + margin), exposed as `Recommendation` status.
- Applies via **in-place resize** (no restarts) – Kubernetes VPA historically had to evict
  pods to change requests; in-place resize removes that pain.
- Modes: `Off` (recommend only), `Initial` (apply at pod creation), `InPlace`.
- Coexistence with horizontal scaling on the same metric is unstable (both react to CPU) –
  validation rejects conflicting configs.

## Cluster (node) autoscaling

Kubernetes Cluster Autoscaler works with **node groups** (ASGs) and simulates scheduling;
Karpenter provisions individual instances by solving bin-packing for pending pods with
flexible instance types, and consolidates (replaces underused nodes).

Rustwain `[proposed]`:
- **NodeProvider plugin interface** (gRPC): `list_offerings(constraints) → instance types
  (cpu, mem, devices, price, zone, capacity-type)`, `create(offering, config) → node
  join`, `delete(node)`.
- **Provisioner controller** (Karpenter-like): watches unschedulable pods (with the
  scheduler's explanation), runs the scheduler's filter logic against **hypothetical
  nodes** from offerings (reusing scheduler plugins – the scheduler crate must support
  simulation), picks the cheapest set of instances, creates them.
- **Consolidation**: periodically, can pods on node X fit elsewhere (simulation)? If yes,
  drain (budgets respected) and delete X. Can N nodes be replaced by one cheaper node?
- Spot/preemptible handling: interruption notices → drain.
- Providers: `static` (no-op), `libvirt`/`cloud-hypervisor` (local VMs – great for testing
  autoscaling on one machine!), cloud providers as plugins.

## Open questions

- Should usage data be persisted at all? (No – ephemeral; VPA recommender keeps its own
  checkpoints in API objects, small.)
- Activator design for scale-to-zero (connection holding in BPF/agent).

## Build checklist

- [ ] Agent usage summaries + server usage store + `rwctl top`.
- [ ] Scaler controller (resource + PSI metrics, behavior policies).
- [ ] External metrics provider interface.
- [ ] Scheduler simulation API for provisioning.
- [ ] Provisioner + consolidation with a local VM provider.
