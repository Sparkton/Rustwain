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

### Crash-recovery walkthrough: the agent dies at each stage

The rules above are easy to state and easy to get wrong. This section walks one pod
(`uid=7c1e…`, two init containers, one sidecar, one main container, a PVC, one IPv6+IPv4
address) through every stage and asks: *the agent process is SIGKILLed right here –
what is on disk, what is running, and what does the restarted agent do?*

#### Three rules that make every case work

1. **Intent before effect.** Any step with a host side effect first writes an *intent*
   record to the checkpoint DB (`agent.db`, redb, one fsynced transaction), then
   performs the effect, then writes *done*. Recovery treats `intent` without `done` as
   "may or may not have happened" and re-runs the step, which must be idempotent.
2. **Everything on the host is named by pod UID.** cgroup `rustwain.slice/pod-7c1e….scope`,
   netns `/run/rustwain/netns/7c1e…`, mounts under `/var/lib/rustwain/pods/7c1e…/`, BPF
   `endpoints` entries tagged with UID, supervisor socket `/run/rustwain/sup/7c1e….sock`.
   So recovery can always find what exists, and **orphan GC** can always tell whose it is.
3. **Supervisors outlive the agent.** Each pod's supervisor is its own process (not a
   child the agent reaps), holds the namespaces, reaps container processes and records
   their exits with a monotonically increasing `event_seq`. It keeps the last 256 events
   in memory and replays from the agent's `last_acked_seq` on reconnect.

#### Recovery entry point

```
agent start (epoch := epoch + 1, persisted)
  │
  ├─ 1. open agent.db; load PodRecords (state, intents, allocations, last_acked_seq)
  ├─ 2. scan host: cgroups under rustwain.slice, /run/rustwain/sup/*.sock,
  │     /run/rustwain/netns/*, mounts under /var/lib/rustwain/pods/*, pinned BPF
  │     endpoint entries
  ├─ 3. join: for each UID → (record?, host artifacts?)
  │       record + artifacts  → resume at recorded stage (table below)
  │       record, no artifacts→ effects never happened / were lost (reboot): re-run
  │       artifacts, no record→ orphan: tear down (unmount, kill cgroup, delete netns,
  │                             remove BPF entry) — never adopt unknown state
  ├─ 4. reconnect supervisors; replay events since last_acked_seq
  ├─ 5. start API watch; reconcile desired (API) vs recovered (local)
  └─ 6. only now admit *new* pods (allocators are consistent again)
```

Step 6 matters: admitting a new pod before step 3 finishes could hand out an IP or CPU
set that a recovering pod still holds. Kubernetes' kubelet has had several bugs of
exactly this shape (CPU manager state vs running containers, device manager checkpoint
vs DRA).

#### Stage by stage

```
 agent        agent.db               host                   supervisor     API
   │  (A) watch event: pod 7c1e bound to this node                          │
   │◀────────────────────────────────────────────────────────────────────────│
   ✖ crash A: nothing written. Restart re-receives the pod from the watch.   │
   │                                                                          │
   │ admit ok ──▶ PodRecord{Admitted, spec_hash}                              │
   │ status Admitted ───────────────────────────────────────────────────────▶│
   ✖ crash B: record exists, no intents. Resume at Preparing. If spec_hash    │
   │          ≠ API spec (pod replaced with same name) → UID differs anyway.  │
   │                                                                          │
   │ intent{ip} ─▶ bitmap bit set + intent, same txn                          │
   ✖ crash C1: IP reserved in bitmap, not yet programmed. Recovery re-runs    │
   │           ip step: allocate(uid) returns the same IP (keyed by UID).     │
   │ intent{image pull ×4} → pulling (content store dedups by digest)         │
   ✖ crash C2: partial layer downloads exist as *ingest* files; pull resumes  │
   │           with HTTP Range from the ingest offset, digest verified at end.│
   │ intent{mount pvc} ─────────────────▶ mount(…/7c1e/volumes/data)          │
   ✖ crash C3: mount may or may not exist. Recovery checks /proc/self/        │
   │           mountinfo for the exact target + source; mounted → mark done,  │
   │           not mounted → retry. Never mount twice.                        │
   │ done{all prepare} ─▶ PodRecord{Prepared}                                 │
   │                                                                          │
   │ intent{sandbox} ─▶                                                       │
   │ spawn supervisor ─────────────────▶ cgroup + netns + sup socket          │
   ✖ crash D: supervisor may be alive without the agent ever having heard     │
   │          "ready". Recovery finds sup socket → connect → ask state:       │
   │          SandboxReady → done; still starting → wait; socket missing but  │
   │          cgroup exists → supervisor died mid-setup → kill cgroup, delete │
   │          netns, redo sandbox (cheap: < 50 ms).                           │
   │ dataplane: wire netns, endpoints[ip] := {ifindex, uid, identity}         │
   ✖ crash D2: BPF endpoint entry is pinned → survives. Recovery compares     │
   │           pinned entry's ifindex with the netkit device in the netns;    │
   │           mismatch (device recreated) → rewrite entry.                   │
   │                                                                          │
   │ init-1 start ───────────────────────────────────────────▶│ exec          │
   │                                     init-1 exits 0  ◀────│ event seq=12  │
   ✖ crash E: agent never saw seq=12. On reconnect supervisor replays seq≥12  │
   │          → agent records init-1 Completed, starts init-2. An init        │
   │          container is **never re-run after it recorded success**, even  │
   │          if the agent crashed before persisting it (the supervisor's    │
   │          record is authoritative for exits).                            │
   │ sidecar start ─────────────────────────────────────────▶│               │
   ✖ crash E2: sidecar keeps running under the supervisor. Startup probe      │
   │           state is lost (in-memory) → re-probed; init-2 starts only once │
   │           the sidecar's startup probe passes again (idempotent gate).    │
   │                                                                          │
   │ main start; Running ──────────────────────────────────────────────────▶│
   ✖ crash F: containers keep running (supervisor). Probes pause for the      │
   │          duration of the outage (no liveness kills while blind);         │
   │          readiness is **kept at its last value** until the first probe   │
   │          result after restart (not flipped to false → no endpoint churn  │
   │          on every agent upgrade). Exits during the outage are replayed;  │
   │          restart decisions happen on reconnect with backoff computed     │
   │          from the recorded exit timestamps.                              │
   │                                                                          │
   │ delete requested; PodRecord{Terminating, started_at(wall), boot_id,      │
   │                             drain_started_at, grace=30s}                 │
   │ drain gate: wait for dataplanes' observed generation …                   │
   ✖ crash G: resume drain gate with remaining = drainTimeout − elapsed       │
   │          (elapsed from persisted wall time; same boot_id required).      │
   │ preStop → SIGTERM ─────────────────────────────────────▶│               │
   ✖ crash G2: SIGTERM already sent (intent+done recorded) → don't re-run      │
   │           preStop; wait remaining grace = 30 s − elapsed; then           │
   │           cgroup.kill. If only intent was recorded → re-send SIGTERM     │
   │           (idempotent for any well-behaved process).                     │
   │                                                                          │
   │ Cleaning: unmount, release IP (→ quarantine{until}), remove BPF entry,   │
   │           remove netns, kill supervisor, rmdir cgroup                    │
   ✖ crash H: each cleanup step is idempotent ("already gone" = success). The │
   │          IP quarantine deadline is persisted, so a restart doesn't       │
   │          release the IP early.                                           │
   │ PodRecord{Terminated, final_status, reported=false}                      │
   │ final status ─────────────────────────────────────────────────────────▶│
   ✖ crash I: reported=false → re-send final status on restart (status writes │
   │          are idempotent applies). The API object is only deleted after   │
   │          this report, so the pod can't vanish with an unknown exit code. │
   │ ack ◀──────────────────────────────────────────────────────────────────│
   │ delete PodRecord                                                         │
```

#### Summary table

| Crash point | Durable state | Running state | Recovery action | Must never happen |
|---|---|---|---|---|
| A – before admit | none | none | Re-admit from watch | — |
| B – admitted | `Admitted` | none | Start prepare | Admit a second copy (UID-keyed) |
| C – preparing | intents, partial allocations | partial mounts, ingest files | Re-run each unfinished step idempotently | Double mount; new IP for same UID |
| D – sandbox | `intent{sandbox}` | supervisor maybe | Adopt via socket, or tear down + redo | Two supervisors for one UID |
| E – init/sidecar | init progress up to last ack | init/sidecar processes | Replay supervisor events; re-gate on startup probe | Re-run a completed init container |
| F – running | `Running`, restart counts | all containers | Reconnect, resume probes, replay exits | Liveness kill based on stale probe state; readiness flap |
| G – terminating | start time, boot ID, phase of termination | containers in grace | Continue with *remaining* grace | Grace restarting from zero (pod outlives its budget) |
| H – cleaning | quarantine deadlines | leftovers | Idempotent cleanup | Early IP reuse |
| I – terminated, unreported | final status, `reported=false` | nothing | Resend status | Pod object deleted without final status |
| Reboot (boot ID changed) | all records | nothing | Every pod is dead: report `Failed{NodeRebooted}` for `Never` pods, re-create sandboxes for others (pods are *restarted*, not resumed); free all allocations except quarantine | Resuming a "Running" record whose processes no longer exist |

#### Testing it

Each `✖` above is a named **crash point** (`crashpoint!("sandbox.after_spawn")`)
compiled in under a `crash-points` cargo feature. The integration suite iterates over
every registered point: run the pod lifecycle, `abort()` the agent at that point, restart
it, and assert the invariants in the last column plus "the final API state equals the
no-crash run". This is cheap to run in CI (`cargo xtask crash-matrix`) and turns
"every stage is resumable" from a claim into a test (pain point `PP-5G`).

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
