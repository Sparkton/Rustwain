# Pod Lifecycle in Detail

**K8s analogue:** kubelet `syncPod`, pod phases/conditions, init containers, restart
policy, CrashLoopBackOff, lifecycle hooks, termination.

## Phases and conditions

Kubernetes pod `phase` (Pending, Running, Succeeded, Failed, Unknown) is coarse and
confusing (a pod in CrashLoopBackOff is "Running"). Conditions (PodScheduled,
Initialized, ContainersReady, Ready, PodReadyToStartContainers) add detail.

Rustwain keeps a phase for compatibility of mental model but makes the **reason**
first-class and structured:

```rust
pub struct PodStatus {
    pub phase: PodPhase,                   // Pending | Running | Succeeded | Failed | Unknown
    pub stage: PodStage,                   // fine-grained: Scheduling, PullingImages{progress},
                                           // MountingVolumes{pending}, StartingSandbox,
                                           // Initializing{container}, Running, Terminating{since}, ...
    pub conditions: Vec<Condition>,        // Scheduled, Initialized, ContainersReady, Ready, ...
    pub containers: Vec<ContainerStatus>,
    pub blocking: Option<BlockingReason>,  // *why is this pod not progressing?* (see below)
    pub ip: Option<PodIps>,
    pub resources: AppliedResources,       // what cgroups actually have (after in-place resize)
    pub timings: PodTimings,               // admitted_at, image_ready_at, sandbox_ready_at, started_at...
}
```

`blocking` answers the #1 user question ("why isn't my pod running?") directly:
`ImagePullFailed{ref, error, retry_in}`, `VolumeAttachPending{claim, since}`,
`InsufficientResources{resource}`, `CrashLooping{container, last_exit, backoff}`,
`ReadinessFailing{probe, last_output}`, `WaitingForSidecar{name}`.

`timings` makes startup latency analysable per pod – feeds the
[startup latency](../11-efficiency/03-startup-latency.md) work.

## Full sequence

### 1. Receive and admit
- Agent receives pod via watch.
- **Local admission** checks (fast, deterministic):
  - Resources fit (allocatable minus admitted pods' requests) – the scheduler's view may be
    stale.
  - Required node features present (isolation class, kernel features).
  - Devices allocated by scheduler are present and healthy.
  - Host ports free.
  - Security: tenant policy permits requested privileges on this node.
  - Topology: CPU pinning/NUMA feasible.
- Reject → pod `Failed` with reason `NodeAdmissionRejected{...}`; the owning controller
  will create a replacement (and scheduler gets a hint to avoid the node).

### 2. Prepare (parallel)
Run concurrently, each with timeouts and status reporting:
- **Images**: pull all container images (parallel, deduplicated).
- **Volumes**: wait for attach (controller-side), then mount/format/stage on node.
- **Network**: allocate IP(s) (local IPAM from node CIDR – no API round trip).
- **Devices**: `prepare` allocations.
- **Secrets/config**: materialise projected files into a pod tmpfs.
- **UID range**: allocate.

Kubernetes does many of these sequentially (volumes before sandbox, images per container
just before start). Parallelising is a direct latency win.

### 3. Sandbox
- Spawn supervisor → namespaces, cgroup → dataplane wires netns.
- Pod condition `SandboxReady` (≈ `PodReadyToStartContainers`).

### 4. Init and sidecar ordering
Ordered list from spec:
- **Init containers** run one at a time, each to successful completion. Failure → restart
  per pod restart policy (with backoff); with `restartPolicy: Never`, the pod fails.
- **Sidecar containers** (Kubernetes 1.28+ "restartable init containers": an init
  container with `restartPolicy: Always`): started in order among init containers; the
  next init step proceeds once the sidecar is *started* (and passes its startup probe if
  defined); they keep running during the main phase; they're **stopped after** main
  containers on termination. See [sidecars](08-sidecars-and-debugging.md).

### 5. Main containers
- Started in parallel (Kubernetes behaviour) by default.
- Optional `startOrder`/`dependsOn` between main containers `[proposed]` – a frequently
  requested Kubernetes feature (e.g. app waits for a proxy) that Kubernetes resolves only
  via sidecars. Having both is redundant; lean: sidecars cover most cases, keep the API
  smaller.
- `postStart` hook: runs after the container process is created; Kubernetes semantics:
  the container isn't marked Running until postStart completes; if it fails, the container
  is killed. Keep the same.

### 6. Running
- Restart policy per pod (`Always`, `OnFailure`, `Never`) and per container override
  (Kubernetes 1.34 alpha added per-container restart policy & rules on exit codes).
  Rustwain: per-container policy with **exit-code rules** from day one:
  ```yaml
  restart:
    policy: OnFailure
    rules: [ { exitCodes: [42], action: Restart }, { exitCodes: [0], action: Complete } ]
  ```
- **Backoff**: exponential, 10 s base doubling to a 5 min cap (Kubernetes' defaults;
  1.33+ reduced initial backoff options exist). Rustwain: base 1 s, cap 2 min, **reset
  after the container runs for 10 min** (same as kubelet's reset rule) – and backoff state
  visible in status with `next_restart_at`. Tunable per workload.
- Probes drive readiness and liveness restarts – see [health probes](03-health-probes.md).

### 7. Termination
Triggered by deletion (API sets `deletionTimestamp` + grace period), eviction, or
completion.

```
t=0          pod marked terminating (status updated → endpoints removal starts)
             ├─ readiness forced false for this pod immediately
             └─ (optional) preStop delay: `sleep` action without exec (KEP-3960)
t=0..g       for each main container concurrently:
               preStop hook (exec/http/sleep) → stop signal → wait
t=g          SIGKILL via cgroup.kill for remaining main containers
then         sidecars stopped in reverse order (each with its own remaining grace, min 2s)
then         cleanup: unmount volumes, release IP/devices/UID range, remove sandbox
then         final status written; API object deleted (by agent's confirmation: the pod is
             only removed from the store once the agent reports terminated, or the node is
             declared dead and force-deleted)
```

#### The endpoint-removal race
Classic Kubernetes problem: a pod receives SIGTERM at the same time endpoints are
removed; load balancers and other nodes' kube-proxies take time to stop sending traffic,
so the app shuts down while still receiving requests → errors during every rollout.
Workaround everyone uses: `preStop: sleep 5–15`.

Rustwain approach:
1. When termination starts, the endpoint controller removes the pod **first**, and every
   node's dataplane acks the endpoint map update (via a per-endpoint-set **observed
   generation** reported in node status `[proposed]`).
2. The agent delays sending SIGTERM until either (a) all dataplanes have observed the
   removal, or (b) a `drainTimeout` (default 5 s) passes.
3. For external LBs, the cloud controller reports deregistration completion where the API
   supports it.
This makes graceful shutdown correct by default instead of by folklore. Cost: termination
takes slightly longer. Tracked in the [pain point tracker](../12-critique/06-pain-point-tracker.md).

#### Termination messages
- On exit, read `/dev/termination-log` (path configurable) up to 4 KiB, or last N log lines
  if `fallbackToLogsOnError`. Stored in container status.

### 8. Deletion of the API object
- Graceful: agent confirms → API deletes.
- Node unreachable: pods stay `Terminating` forever in Kubernetes until the node returns or
  someone force-deletes – a common confusion. Rustwain: node lifecycle controller marks
  them `Unknown` with `NodeUnreachable` reason; replacements are created by owners per the
  workload's tolerance (StatefulSets wait for fencing). See
  [HA and failure modes](../10-cluster-operations/03-ha-and-failure-modes.md).

## Pod UID and identity
- A pod is identified by UID; a recreated pod with the same name has a new UID. All
  node-local state is keyed by UID, never by name, to avoid mixing old and new incarnations
  (a real class of kubelet bugs).

## Static and mirror pods
See [node agent architecture](01-node-agent-architecture.md).

## Restart of the agent mid-lifecycle
Every stage is resumable from the checkpoint:
- Preparing: re-run idempotent prepare steps (pull dedups against store, mount checks
  existing mounts, IPAM returns the same allocation for the same UID).
- Sandbox/containers: recovered from supervisors.
- Terminating: resume with *remaining* grace computed from the persisted start time
  (monotonic-ish: stored as wall time + boot ID; if boot ID changed, the pod is dead
  anyway).

## Open questions

- Container start ordering among main containers – include or not?
- Should `Succeeded`/`Failed` pods be deleted from the API automatically after a TTL by
  default (Kubernetes: only Jobs with `ttlSecondsAfterFinished`)? Lean: owner-specific.

## Build checklist

- [ ] `PodStage`, `BlockingReason`, `PodTimings` types.
- [ ] Parallel prepare executor with per-step timeouts and status updates.
- [ ] Init/sidecar/main ordering engine with tests for all orderings.
- [ ] Restart policy with exit-code rules + backoff (simulated clock tests).
- [ ] Termination sequencer with endpoint-drain gate.
- [ ] Agent-restart-at-every-stage test (crash injection points).
