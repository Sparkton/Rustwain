# Resources, QoS and Eviction

**K8s analogue:** requests/limits, QoS classes (Guaranteed/Burstable/BestEffort), node
allocatable (kube-reserved/system-reserved), CPU manager, memory manager, topology
manager, hugepages, eviction manager, in-place pod resize, swap support, pod overhead.

## Resource model

| Resource | Request means | Limit means | Implementation |
|---|---|---|---|
| CPU | Guaranteed share under contention; scheduling input | Hard cap (quota) – **optional** | `cpu.weight` / `cpu.max` (+burst) |
| Memory | Scheduling input; protection from reclaim | Hard cap → OOM kill | `memory.min`/`low`, `memory.high`, `memory.max` |
| Ephemeral storage | Scheduling input | Eviction (or quota enforcement) | Project quotas / scan |
| Hugepages-<size> | Must equal limit | Same | `hugetlb.*.max` |
| PIDs | – | Per-pod cap | `pids.max` |
| Devices | Allocation | – | See devices doc |
| Network bandwidth | `[proposed]` scheduling input | Egress/ingress rate | EDT in eBPF |
| IO | `[open]` | `io.max` | Opt-in |

Units are typed (`MilliCpu(u64)`, `Bytes(u64)`); string parsing (`500m`, `1Gi`) only at
API edges, with the exact Kubernetes quantity grammar for familiarity.

## Node allocatable

```
Capacity (hardware)
 − system reserved     (OS daemons: sshd, journald, …)
 − rustwain reserved   (agent + supervisors' fixed overhead – measured, not guessed)
 − eviction threshold  (hard eviction reserve)
 = Allocatable         (what the scheduler can hand out)
```

Kubernetes defaults reserve nothing unless configured, so many clusters oversubscribe
the node and kubelet/runtime get OOM-killed under pressure. Rustwain defaults compute
reserves from node size (similar to GKE/EKS formulas: e.g. memory: 25% of first 4 GiB,
20% of next 4, 10% of next 8, 6% of next 112, 2% above; CPU: 6% of first core, 1% of
next, 0.5% of next 2, 0.25% above) and **enforce** them via the cgroup tree
(`rustwain-system` gets `memory.min` and a CPU weight).

## QoS classes

Derived from requests/limits exactly as Kubernetes does (kept for familiarity):
- **Guaranteed**: every container has requests == limits for CPU and memory.
- **Burstable**: some requests set.
- **BestEffort**: nothing set.

Mapping to cgroups:

| QoS | cpu | memory | OOM priority (`oom_score_adj`) | Swap |
|---|---|---|---|---|
| Guaranteed | weight from request; optional exclusive cpuset | `memory.min = request` | −997 (protected) | none |
| Burstable | weight from request | `memory.low = request`, `memory.high ≈ 0.9×limit`, `memory.max = limit` | scaled 2..999 by request/capacity | allowed proportional to request (`LimitedSwap`) |
| BestEffort | `cpu.idle = 1` (SCHED_IDLE) | none | 1000 (first to die) | none |

Note: with `memory.oom.group` and cgroup-aware OOM, the kernel's choice is mostly
driven by the memory cgroup hitting its own `memory.max`; node-level OOM (global) uses
`oom_score_adj` – we also run **userspace OOM prevention** (below) so global OOM should
be rare.

## CPU management

### Shared pool (default)
All non-pinned pods share all non-reserved CPUs, with weights. Limits optional (see
cgroups doc on throttling).

### Exclusive CPUs (static policy)
Guaranteed pods with **integer** CPU requests can get exclusive cores
(`cpuset.cpus`), removed from the shared pool.

Kubernetes' CPU manager `static` policy has rough edges:
- It's a node-level setting requiring kubelet restart and state file deletion to change.
- Allocation can fragment; SMT siblings split across pods unless `full-pcpus-only`.
- NUMA alignment requires topology manager, which may reject pods at admission after
  scheduling.

Rustwain:
- Exclusive CPUs are a **pod request** (`cpuPolicy: exclusive`) the scheduler knows about
  (node publishes free exclusive CPUs per NUMA node), so admission failures are rare.
- Allocation is topology-aware: prefer whole physical cores (both SMT siblings), same NUMA
  node as allocated devices and memory, avoid efficiency cores on hybrid CPUs.
- Uses `cpuset.cpus.partition = isolated` where available to also exclude these CPUs from
  load balancing.
- Housekeeping CPUs (for the agent, IRQs, kernel threads) reserved explicitly.

### IRQ and kernel noise (advanced, opt-in)
For latency-critical pods: steer IRQs away from exclusive CPUs (`/proc/irq/*/smp_affinity`),
disable timer ticks (`nohz_full` is boot-time only – report, can't change), set CPU
frequency governor. This is what OpenShift's "performance profile" automates. `[later]`.

## Memory management

### NUMA
- Guaranteed pods may request `numa: single` → `cpuset.mems` restricted to the NUMA
  node(s) matching their CPUs.
- Hugepages allocated per NUMA node; the scheduler must know per-node free hugepages.

### Swap
Kubernetes historically required swap off (kubelet refused to start). Since 1.28+
`NodeSwap` with `LimitedSwap` for Burstable pods. Swap helps density and cushions
spikes, but makes latency unpredictable.

Rustwain:
- Swap allowed on nodes; per-QoS policy: Guaranteed – none; Burstable –
  `memory.swap.max` proportional to request (Kubernetes' LimitedSwap formula); BestEffort
  – none (or all, configurable).
- **zswap/zram** strongly recommended (compressed in-memory swap) – detect and report.
- Swap usage visible in pod status.

### Proactive reclaim
For density: periodically write to `memory.reclaim` for pods whose PSI memory pressure is
zero and that have cold page cache, reclaiming idle cache before the node is under
pressure (Meta's Senpai/TMO approach). Off by default `[proposed]`, on in "dense" profile.

## In-place resize

Change CPU/memory requests and limits of a running pod without restart (Kubernetes
1.27 alpha → 1.33 beta → GA). Rustwain: day-one feature – it's cgroup writes:
- CPU: always in place.
- Memory increase: in place. Memory **decrease** below current usage: must not OOM-kill
  – set `memory.high` first to apply pressure, wait for usage to drop, then lower `max`;
  fail the resize with `ResizeInfeasible{current_usage}` after a timeout.
- Per-container `resizePolicy` (`NotRequired` / `RestartContainer`) for apps that read
  limits only at startup (JVM heap).
- Scheduler accounting: the node's accounted requests change only when the agent
  confirms resize (status `resources.applied`).
- MicroVMs: CPU hotplug and virtio-mem; slower, may require guest cooperation.

## Pod overhead
Automatically added per isolation class (see
[isolation spectrum](../03-virtualization/01-isolation-spectrum.md)) and per-pod fixed
overhead (supervisor ~1 MB, netns ~few hundred KB of kernel memory). Accounted in pod
cgroup and scheduler math.

## Eviction

When a node runs short of a non-compressible resource (memory, disk, PIDs), the agent
must evict pods before the kernel OOM killer or a full disk causes chaos.

### Kubernetes' approach and problems
- Signals: `memory.available` (capacity − working set, polled every 10 s by default via
  cAdvisor → slow), `nodefs.available`, `imagefs.available`, `pid.available`.
- Hard thresholds (immediate) and soft thresholds (with grace periods).
- Problems: polling too slow for fast memory spikes (kernel OOM wins the race);
  working-set accounting includes active page cache → false pressure; ranking by usage
  over request punishes the wrong pods sometimes; eviction of a pod doesn't free memory
  until it terminates (grace period!).

### Rustwain approach
1. **PSI triggers** (memory `some`/`full` thresholds on the root and QoS cgroups) wake the
   eviction manager immediately – event-driven, no polling.
2. **Signal quality**: memory pressure = PSI + anon/unreclaimable usage, not raw usage.
3. **Graduated response**:
   - Level 1: proactive reclaim of BestEffort/Burstable page cache (`memory.reclaim`).
   - Level 2: throttle BestEffort (`memory.high` lowered, `cpu.idle`).
   - Level 3: evict, ranked by: (a) QoS (BestEffort first), (b) priority class, (c) usage
     above request (largest first).
   - Level 4 (hard emergency): evict with **zero grace period** (cgroup.kill) to free memory
     *now*.
4. **Disk**: project-quota usage per pod is O(1) to read → evict top consumers above
   request; image GC first (see [image and disk GC](07-image-and-disk-gc.md)).
5. **PIDs**: per-pod `pids.max` makes node PID exhaustion unlikely; still watch
   `kernel.pid_max` headroom.
6. Evicted pods get status `Failed`, reason `Evicted{resource, usage, request}` and the
   owner replaces them; the node gets a `MemoryPressure`/`DiskPressure` condition +
   taint so the scheduler stops placing BestEffort/Burstable pods there.

### API sketch: the eviction manager

```rust
/// Event sources. PSI triggers are pollable fds; disk and PID signals are cheap reads.
pub enum Signal {
    MemoryPressure { scope: CgroupScope, some_avg10: f32, full_avg10: f32 },
    MemoryAvailable { anon_unreclaimable: Bytes, allocatable: Bytes },
    DiskUsage { fs: FsId, used: Bytes, capacity: Bytes, inodes_free: u64 },
    PidHeadroom { used: u64, max: u64 },
}

#[async_trait]
pub trait SignalSource: Send {
    /// Resolves when a registered trigger fires or the periodic fallback tick elapses
    /// (1 s, only used when PSI triggers are unavailable – see the kernel feature matrix).
    async fn next(&mut self) -> Signal;
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub enum Level { Normal = 0, Reclaim = 1, Throttle = 2, Evict = 3, Emergency = 4 }

pub struct Thresholds {
    /// Enter level L when the expression holds for `sustain`; leave it only when the
    /// *exit* expression holds for `cooldown` (hysteresis; prevents flapping).
    pub memory: [LevelRule; 4],
    pub disk: [LevelRule; 2],         // Evict, Emergency
    pub pids: [LevelRule; 1],         // Evict
}

pub struct LevelRule { pub enter: Expr, pub exit: Expr, pub sustain: Duration, pub cooldown: Duration }

/// Ranking: derive(Ord) on field order — first field dominates.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
pub struct VictimKey {
    pub critical: bool,               // node-critical pods sort last (false < true)
    pub qos: QosRank,                 // BestEffort < Burstable < Guaranteed
    pub priority: i32,                // lower priority evicted first
    pub disruption_blocked: bool,     // [proposed] pods whose owner has budget headroom (false) first
    pub over_request: Reverse<i64>,   // larger usage-above-request first
    pub started_at: Reverse<WallTime>,// tie-break: youngest first (least work lost)
}

/// Side effects, behind a trait so the policy is unit-testable with a fake.
pub trait Actuator {
    fn reclaim(&mut self, pod: PodUid, bytes: Bytes) -> io::Result<()>;   // memory.reclaim
    fn throttle(&mut self, pod: PodUid, memory_high: Bytes) -> io::Result<()>;
    fn unthrottle(&mut self, pod: PodUid) -> io::Result<()>;
    /// `grace == ZERO` → cgroup.kill immediately (Emergency).
    fn evict(&mut self, pod: PodUid, reason: EvictionReason, grace: Duration);
}

pub struct EvictionManager<S: SignalSource, A: Actuator> {
    signals: S,
    actuator: A,
    thresholds: Thresholds,
    level: LevelState,                // current level per resource + entered_at
    in_flight: HashMap<PodUid, Instant>,  // evictions not yet finished terminating
    pods: Arc<PodUsageView>,          // per-pod usage from cgroup files, refreshed on signal
}

impl<S: SignalSource, A: Actuator> EvictionManager<S, A> {
    pub async fn run(mut self, clock: impl Clock) -> ! {
        loop {
            let sig = self.signals.next().await;
            let target = self.level.evaluate(&self.thresholds, &sig, clock.now());
            match target {
                Level::Normal   => self.relax(),
                Level::Reclaim  => self.reclaim_cold_cache(),
                Level::Throttle => self.throttle_best_effort(),
                Level::Evict    => {
                    // At most one new eviction per resource until the previous one has
                    // released its memory (or 10 s passed) — avoids evicting 5 pods for
                    // a problem one eviction solves, the over-eviction failure mode.
                    if self.in_flight.is_empty() { self.evict_one(Grace::PodSpec) }
                }
                Level::Emergency => self.evict_one(Grace::Zero), // no in-flight gate
            }
        }
    }
}
```

Design points the sketch encodes:
- **Time is injected** (`Clock`) so level transitions, sustain/cooldown and in-flight
  gates are tested with a simulated clock – no sleeping tests.
- **One eviction in flight** at `Evict` level: kubelet can evict several pods in quick
  succession because a terminating pod still holds its memory during its grace period.
  `Emergency` bypasses the gate and uses zero grace, because by then the kernel OOM
  killer is the alternative.
- **`disruption_blocked` in the ranking** `[proposed]`: node-pressure eviction can't *wait* for
  disruption budgets (Kubernetes ignores PDBs here entirely), but among otherwise equal
  candidates it can prefer pods whose owner currently has budget headroom (`PP-5F`).
- `PodUsageView` reads `memory.stat` (anon, file, unevictable), `memory.current` and the
  pod's project-quota usage only when a signal fires – no background scraping.

### Userspace OOM killer?
systemd-oomd and Meta's oomd kill based on PSI before the kernel OOM. Our eviction
manager *is* a PSI-based userspace OOM preventer scoped to pods. Ensure systemd-oomd
(enabled by default on Fedora/Ubuntu desktop flavours) doesn't also act on our slice:
set `ManagedOOMMemoryPressure=` off for `rustwain.slice` (we own it).

## Pod-level resources
Kubernetes 1.32+ (alpha) adds pod-level requests/limits (shared budget across
containers). Rustwain: supported from day one – the pod cgroup can carry limits;
containers may omit theirs and share. Simplifies sidecar sizing ("this pod gets 2 CPU,
1 GiB, split however").

## Open questions

- Default for CPU limits in lint: warn when set without burst? Many orgs mandate limits
  for chargeback; we should show throttling, not forbid.
- Should BestEffort pods exist in multi-tenant clusters at all? Tenant policy can forbid.

## Build checklist

- [ ] Allocatable calculator with measured agent overhead.
- [ ] QoS → cgroup mapping; unit tests per QoS.
- [ ] Exclusive CPU allocator (topology-aware) with scheduler-visible inventory.
- [ ] In-place resize including safe memory decrease.
- [ ] PSI-driven eviction manager with graduated levels; chaos test with memory hog pods.
- [ ] Swap policy per QoS.
