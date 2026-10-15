# cgroups v2

**K8s analogue:** kubelet's cgroup manager, CPU manager, memory manager, QoS classes and
eviction signals; runc's resource application.

cgroups (control groups) are the kernel's mechanism for **accounting** and **limiting**
resources for groups of processes. Every resource guarantee Rustwain makes is
implemented here.

Decision `[decided]`: **cgroup v2 (unified hierarchy) only.** No v1 or hybrid support.
Why: v1 has per-controller hierarchies that disagree with each other, no reliable
memory+IO coupling (writeback was not charged correctly), no PSI, no safe delegation,
and broken OOM semantics for groups. Every mainstream distro defaults to v2 now
(Fedora 31+, Ubuntu 21.10+, Debian 11+, RHEL 9+, Arch, Alpine 3.19+ with OpenRC unified
mode). Kubernetes itself moved cgroup v1 to maintenance mode in 1.31. RHEL 8 and Amazon
Linux 2 default to v1 – those hosts must be switched to v2 via kernel cmdline
(`systemd.unified_cgroup_hierarchy=1`) or are unsupported. The preflight check
detects this and prints the exact fix.

## The model

- A single tree mounted at `/sys/fs/cgroup` (type `cgroup2`).
- Each directory is a cgroup. Processes live in exactly one cgroup.
- **No internal processes rule:** a non-root cgroup that has controllers enabled for its
  children (`cgroup.subtree_control`) cannot itself contain processes. This forces a
  clean "leaves hold processes, interior nodes hold policy" structure.
- Controllers are enabled top-down: a controller is available in a cgroup only if its
  parent lists it in `cgroup.subtree_control`.
- **Delegation:** a subtree can be handed to a less-privileged owner by chowning
  specific files (`cgroup.procs`, `cgroup.subtree_control`, `cgroup.threads`). Used for
  rootless mode and for nested runtimes.

## Controllers we use

### cpu
| File | Meaning | Rustwain use |
|---|---|---|
| `cpu.weight` (1–10000, default 100) | Proportional share under contention | Derived from CPU **request** |
| `cpu.max` (`$QUOTA $PERIOD`) | Hard cap: quota µs per period µs | Derived from CPU **limit** (optional) |
| `cpu.max.burst` (5.14+) | Allows accumulating unused quota for bursts | Used to soften throttling |
| `cpu.stat` | `usage_usec`, `nr_throttled`, `throttled_usec` | Throttling visibility |
| `cpu.pressure` | PSI | Contention detection |
| `cpu.idle` (5.15+) | Mark group as SCHED_IDLE | Best-effort batch pods |
| `cpu.uclamp.min/max` | Utilization clamping (EAS/ARM big.LITTLE) | `[open]` edge devices |

**The CFS throttling problem.** CPU limits via `cpu.max` are enforced per period
(default 100 ms). A multithreaded app with a 1-CPU limit can burn its whole quota in
25 ms across 4 threads and then be stalled for 75 ms – terrible tail latency even
though average usage is under the limit. Kubernetes users routinely remove CPU limits
because of this. Rustwain approach:

1. CPU limits are **optional and off by default**; requests (weights) provide fairness.
2. When limits are set, enable `cpu.max.burst` (up to the limit's quota) where
   supported.
3. Offer a shorter period option (e.g. 20 ms) for latency-sensitive pods – costs more
   scheduler overhead.
4. Surface `nr_throttled`/`throttled_usec` in pod status so users can *see* throttling.

Mapping from request to weight: Kubernetes maps millicores to v1 `cpu.shares`
(1024 per core), then converts to v2 weight with a formula that compresses the range
(`1 + ((shares - 2) * 9999) / 262142`). We use the same mapping so the semantics are
familiar, but compute it in one typed place.

### cpuset
- `cpuset.cpus`, `cpuset.mems`: pin to specific CPUs and NUMA nodes.
- `cpuset.cpus.partition` (`root`/`isolated`, 5.15+/6.x): create exclusive partitions so
  pinned workloads don't share CPUs with anyone, including kernel housekeeping where
  combined with `isolcpus`/`nohz_full`.
- Used by the CPU manager for "guaranteed, integer CPUs" pods. See
  [resources, QoS, eviction](../04-node-agent/04-resources-qos-eviction.md).

### memory
| File | Meaning | Rustwain use |
|---|---|---|
| `memory.max` | Hard limit → OOM kill when exceeded and unreclaimable | Memory **limit** |
| `memory.high` | Throttle point: reclaim aggressively and slow allocation above it | Set slightly below max to get pressure before OOM |
| `memory.low` | Best-effort protection from reclaim | Burstable pods' requests |
| `memory.min` | Hard protection from reclaim | Guaranteed pods' requests |
| `memory.swap.max` | Swap limit | Per-QoS swap policy |
| `memory.oom.group` | OOM kills whole cgroup together | **On** for containers (no half-dead process trees) |
| `memory.events` | `low`, `high`, `max`, `oom`, `oom_kill` counters | OOM detection without polling dmesg |
| `memory.stat` | anon, file, kernel, slab, sock, shmem, … | Accurate working-set computation |
| `memory.pressure` | PSI | Eviction signal |
| `memory.reclaim` (5.19+) | Proactive reclaim on write | Idle memory reclaim, density |
| `memory.peak` (5.19+) | High-water mark | Right-sizing recommendations |

**The page cache problem.** Page cache from file IO is charged to the cgroup. Kubernetes'
"working set" = usage − inactive_file, which still counts active file pages, so apps
doing heavy IO look like they're near their limit and get evicted even though the
cache is reclaimable. Rustwain:

- Uses `memory.stat` (anon + kernel + active non-reclaimable) and **PSI** to decide
  pressure, not raw usage.
- Uses `memory.high` so the kernel reclaims cache *before* an OOM, and `memory.reclaim`
  for proactive trimming of idle pods.

### io
- `io.weight` / `io.bfq.weight`: proportional IO (needs BFQ or `io.cost`).
- `io.max`: `rbps`, `wbps`, `riops`, `wiops` per device (major:minor).
- `io.latency`: latency target protection.
- `io.cost.qos` / `io.cost.model` (root only): the iocost controller – the best
  proportional IO controller in the kernel, but needs per-device model tuning.
- `io.pressure`: PSI.
- **Writeback is charged correctly only in v2** (with memory controller enabled) – the
  core reason v1 IO limits never worked for buffered writes.

Decision `[proposed]`: IO limits are opt-in per pod (`io.max` on the device backing
ephemeral storage); default fairness via `io.weight` when the device's scheduler
supports it. Detect scheduler (`/sys/block/*/queue/scheduler`) in the host probe.

### pids
- `pids.max`: fork bomb protection. Default per pod: 4096 (configurable). Kubernetes
  leaves this unset by default; we set it.

### hugetlb
- `hugetlb.<size>.max`, `.rsvd.max`: limits for 2 MiB / 1 GiB hugepages.

### rdma, misc
- `rdma.max`: HCA handles/objects.
- `misc.max`: e.g. SEV/SEV-ES ASIDs for confidential VMs (`misc.capacity` lists them).

### Device access
cgroup v2 has **no devices controller files**. Device access is controlled by attaching
an eBPF program of type `BPF_PROG_TYPE_CGROUP_DEVICE` to the cgroup. Rustwain's
supervisor generates and attaches this program per container from the allowed device
list (via `aya`), the same way runc/crun do.

### freezer
- `cgroup.freeze` (5.2+): freeze/thaw all processes. Used for pause/resume,
  consistent snapshots, and race-free kill (`cgroup.kill`).
- `cgroup.kill` (5.14+): write `1` to SIGKILL every process in the cgroup atomically,
  including ones forking concurrently. **This is how Rustwain kills containers** – no
  racy PID-list iteration.

## PSI – Pressure Stall Information

`{cpu,memory,io}.pressure` files report:

```
some avg10=0.00 avg60=0.00 avg300=0.00 total=0
full avg10=0.00 avg60=0.00 avg300=0.00 total=0
```

- `some`: % of time at least one task was stalled on the resource.
- `full`: % of time *all* non-idle tasks were stalled (no progress at all).
- **PSI triggers**: write `"some 150000 1000000"` to the file, then `poll()` for
  `POLLPRI` – the kernel wakes you when 150 ms of stall occurs in any 1 s window. No
  polling loop needed.

Rustwain uses PSI triggers at the node level and per pod as the primary eviction and
"noisy neighbour" signal (see [resources, QoS, eviction](../04-node-agent/04-resources-qos-eviction.md)).
Requires `CONFIG_PSI=y`; some distros need `psi=1` on cmdline (it's built but
default-off on some RHEL kernels – `CONFIG_PSI_DEFAULT_DISABLED`). The probe checks.

## Hierarchy layout

```
/sys/fs/cgroup/
├─ system.slice/                         (systemd services, incl. rustwain-agent.service)
├─ user.slice/
└─ rustwain.slice/                       (delegated to rustwain by systemd)
   ├─ rustwain-system/                   (agent, supervisors' own overhead – reserved)
   ├─ guaranteed/
   │  └─ pod-<uid>/
   │     ├─ supervisor/                  (supervisor process – charged to pod overhead)
   │     ├─ c-app/                       (container leaf)
   │     └─ c-sidecar/
   ├─ burstable/
   │  └─ pod-<uid>/...
   └─ besteffort/
      └─ pod-<uid>/...
```

- QoS tiers are interior nodes with policy: `besteffort` gets minimal `cpu.weight` and no
  memory protection; `guaranteed` pods get `memory.min`.
- **Pod-level cgroup** enforces the pod total (including overhead for VMs) – useful for
  pod-level limits and accounting.
- Supervisor charged to the pod so its (tiny) memory counts against the pod, not the
  node.

## systemd interplay

Two cgroup drivers in Kubernetes terms: `cgroupfs` (write directly) and `systemd` (ask
systemd over D-Bus to create scopes/slices). Kubernetes requires them to match between
kubelet and runtime, and mismatches are a classic misconfiguration.

Decision `[decided]`: **one driver, chosen automatically:**
- On systemd hosts, `rustwain-agent.service` sets `Delegate=yes` and
  `DelegateSubgroup=` (systemd 254+) or we create `rustwain.slice` with delegation.
  systemd then promises never to touch anything below it, and we write cgroupfs
  directly within our delegated subtree. No D-Bus scope creation per container – it is
  slow (tens of ms per call) and makes systemd a bottleneck at high pod churn.
- On non-systemd hosts (Alpine/OpenRC, minimal images), we own the tree directly
  under a top-level `rustwain` cgroup.

## Reading stats efficiently

- Avoid reading dozens of files per container per second. Batch reads per pod, use
  `cpu.stat` and `memory.stat` (single read each), and read on demand when status is
  requested or at a coarse interval (10 s).
- For high-frequency needs (autoscaling signals), attach an eBPF program to
  `cgroup` tracepoints or use `memory.events` with `inotify`/`poll` (modifications to
  `*.events` files generate file-modified notifications) instead of polling.

## Rust implementation notes

- A `cgroup` crate inside `rustwain-sys` with typed controllers:
  `CpuMax { quota: Option<Duration>, period: Duration }`, `MemoryLimits { min, low, high,
  max, swap_max }`, etc. Writes are validated against kernel ranges before hitting the
  filesystem.
- Prefer `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` relative to an fd of the pod
  cgroup dir to avoid path races.
- Existing crates: `cgroups-rs` (supports v1+v2, heavier), youki's `libcgroups`. We will
  likely write a small v2-only module – the file format is simple and owning it keeps
  dependencies minimal.

## Open questions

- Use `memory.high` at what ratio of `memory.max`? 0.9 is a common heuristic; needs
  experimentation (too low causes needless throttling).
- Should best-effort pods use `cpu.idle`? Probably yes – it's strictly better than weight=1.

## Build checklist

- [ ] Host probe: cgroup2 mounted, controllers available at root, delegation status, PSI
      enabled, `cgroup.kill`/`memory.reclaim`/`cpu.max.burst` support.
- [ ] Typed writers per controller, unit-tested against a tmpfs fake.
- [ ] Integration tests on real kernel: OOM group kill, `cgroup.kill`, PSI trigger fires.
- [ ] Device eBPF program generator + test that a denied device returns `EPERM`.
