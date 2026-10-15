# Node Agent Architecture

**K8s analogue:** kubelet + kube-proxy + container runtime daemon + CNI daemon +
NodeLocal DNSCache + CSI node plugin host + node-problem-detector.

## Responsibilities

The agent owns everything on one machine:

1. Register the node, maintain its lease, publish capacity and features.
2. Watch pods bound to this node; admit or reject them locally.
3. Pull images, prepare volumes, allocate IPs and devices.
4. Create, start, monitor, restart and stop pods via supervisors.
5. Run probes and report status.
6. Enforce resource pressure policy (eviction).
7. Program the network dataplane (pod interfaces, service LB, policy, encryption).
8. Serve node-local DNS.
9. Garbage-collect images, containers, logs.
10. Serve streaming endpoints (logs, exec, attach, port-forward) to the API.

## Internal structure

```
rustwain agent
├─ core
│  ├─ config            (static file + flags + dynamic config object from API)
│  ├─ host probe        (HostFeatures at start; re-probe on demand)
│  ├─ api client        (mTLS gRPC; watch streams; status writer with batching)
│  ├─ checkpoint store  (redb at /var/lib/rustwain/agent.db)
│  └─ event bus         (bounded broadcast of internal events)
│
├─ pod manager                       ← the kubelet core
│  ├─ source merger     (API pods + static pods dir)
│  ├─ admission         (resources, features, security, devices)
│  ├─ pod workers       (one async task per pod; owns its state machine)
│  ├─ probe manager
│  ├─ status manager    (coalesces and writes status)
│  └─ eviction manager  (PSI + thresholds)
│
├─ runtime              (RuntimeBackend impls; supervisor spawning/recovery)
├─ image store          (pull, unpack, GC)
├─ volume manager       (attach/mount/format, projected volumes, quotas)
├─ device manager       (DeviceDrivers, allocations)
├─ resource manager     (cgroup tree, CPU/memory/NUMA pinning)
├─ dataplane            (eBPF/nftables programs, IPAM, interfaces, services, policy, WireGuard)
├─ dns                  (node-local resolver)
└─ stream server        (exec/attach/port-forward/logs, authenticated by API-issued tokens)
```

All modules are tokio tasks communicating via typed bounded channels and shared
read-mostly state (`Arc<ArcSwap<…>>` snapshots). No module calls another module's
internals directly; they go through narrow handles.

## The pod worker model

Kubelet's core design (`podWorkers`, `syncPod`) is: one goroutine per pod, level-triggered
`syncPod` that compares desired spec to runtime state and takes one step. Kubelet's
implementation accreted complexity (`SyncPod`, `SyncTerminatingPod`,
`SyncTerminatedPod`, multiple caches – the PLEG (Pod Lifecycle Event Generator)
relisting all containers every second was a notorious scalability issue; "PLEG is not
healthy" errors at high density). Event-based PLEG with CRI events arrived later.

Rustwain:
- **One task per pod** owning a `PodState` machine.
- Inputs (merged into a single `select!` loop): spec updates from API, supervisor events
  (pushed, no relisting), probe results, eviction decisions, timer ticks for backoff.
- Each iteration: compute `desired actions = f(spec, observed state)`, execute the
  next action, persist checkpoint if state changed, publish status if changed.
- **No global relist.** Supervisors push events; the agent never polls the runtime. A
  slow periodic consistency check (every 5 min) detects drift.

### Pod state machine (simplified)

```
          ┌─────────┐
          │ Pending │ (received, not admitted)
          └────┬────┘
               │ admit ok              admit fail → Rejected (terminal, status reason)
               ▼
        ┌─────────────┐
        │ Preparing   │ images pulling, volumes mounting, IP allocating, devices preparing
        └──────┬──────┘
               ▼
        ┌─────────────┐
        │ SandboxUp   │ supervisor ready, netns wired
        └──────┬──────┘
               ▼
        ┌─────────────┐
        │ Initializing│ init containers (sequential), restartable sidecars started in order
        └──────┬──────┘
               ▼
        ┌─────────────┐  container exits → per restart policy: restart w/ backoff
        │ Running     │◀──────────────────────────────────┐
        └──────┬──────┘                                   │
               │ deletion requested / all done (Never/OnFailure policy)
               ▼
        ┌─────────────┐
        │ Terminating │ preStop hooks, signals, grace period, sidecars stop last
        └──────┬──────┘
               ▼
        ┌─────────────┐
        │ Cleaning    │ unmount, release IP/devices, remove sandbox, final status
        └──────┬──────┘
               ▼
          Terminated (status persisted to API, then local state removed)
```

Detailed lifecycle: [pod lifecycle](02-pod-lifecycle.md).

## Talking to the API

- **Watch scope**: `pods where spec.node == <me>`, plus objects those pods reference
  (secrets, config maps, claims) – the **node authorizer** on the server enforces that
  the agent can only read objects referenced by its own pods.
- **Secrets and config delivery**: rather than the agent watching every referenced secret
  individually (kubelet uses watch-based or TTL cache per object), the server offers a
  **node bundle stream**: a single watch delivering all objects the node is entitled to,
  computed server-side. `[proposed]` – reduces N watches to 1, and the authorization
  logic lives in one place.
- **Status writes**: coalesced per pod (latest-wins), batched across pods, rate-limited.
  Status is written with **patch/apply of the status subresource** with the agent as field
  manager.
- **Disconnected operation**: if the API is unreachable, everything keeps running;
  restarts continue per policy; status queues (bounded, latest-wins per pod) and flushes
  on reconnect.

## Checkpointing

`/var/lib/rustwain/agent.db` (redb) stores:
- Pods admitted (spec hash, resolved spec reference), allocations (IPs, UID ranges, CPUs,
  devices, ports), volume mount state, image leases.
- Agent epoch (increments per start).

Why: after an agent restart *with the API down*, the agent must know what it was
running, what it allocated, and must not double-allocate. Kubelet keeps some of this in
checkpoint files (CPU manager state, device manager checkpoint, DRA checkpoint) but not
uniformly.

## Static pods

Pods defined by files in `/etc/rustwain/pods.d/*.yaml`, run regardless of API
connectivity. Used to bootstrap control-plane components (if run as pods) and
node-critical daemons. Mirrored to the API as read-only "mirror" objects so they're
visible. `rustwain agent --standalone` = static pods only, no API at all – the
single-node mode for milestone 1.

## Agent configuration

- Static config: `/etc/rustwain/agent.toml` (TOML – readable, typed via serde).
- Dynamic config: a `NodeConfig` object in the API (selected by node labels) for
  tunables like eviction thresholds, max pods, image GC thresholds. Changes apply live
  where safe; unsafe ones (CPU manager policy) require drain + restart and the agent
  reports `ConfigPendingRestart`.

Kubernetes had "Dynamic Kubelet Config" and removed it (1.24) because it was hard to make
safe. We scope dynamic config to tunables that are designed to be live-reloadable.

## Memory and CPU budget for the agent

Target < 40 MB RSS idle with 100 pods (see
[performance budgets](../11-efficiency/01-performance-budgets.md)). Techniques:
- No caching of full API objects beyond this node's pods.
- Compact internal pod representation (interned strings for labels/images).
- Probes scheduled on a timer wheel, not one task per probe.
- Stats read lazily.
- eBPF maps sized to node needs, not cluster size, where possible (service maps are
  cluster-sized by nature – see dataplane doc for compression).

## Failure isolation inside the agent

A bug in one module (e.g. DNS) shouldn't take down pod management. Rust gives us memory
safety, but panics and deadlocks still happen:
- Each module runs under a supervisor task that restarts it on panic (with backoff) and
  marks a node condition (`DnsDegraded`).
- `panic = "unwind"` in the agent (not abort) so module supervision works; `catch_unwind`
  at module task boundaries.
- Watchdog: systemd `WatchdogSec` + `sd_notify(WATCHDOG=1)` from a task that checks every
  module's heartbeat; a wedged agent is restarted (which is safe – see restart design).

## Open questions

- Node bundle stream vs per-object watches (API design impact).
- Do we isolate the DNS server and stream server into separate processes for blast
  radius? They handle untrusted input from pods/network. Lean: yes for DNS
  (`rustwain dns` child process with tight Landlock/seccomp), stream server stays
  in-agent but behind strict auth.

## Build checklist

- [ ] Module skeleton with supervision and heartbeat watchdog.
- [ ] Pod worker state machine with exhaustive transition tests (property tests with
      random event sequences: invariants like "never two sandboxes for one pod UID").
- [ ] Checkpoint store schema + migration story.
- [ ] Standalone mode with static pods directory.
- [ ] API client with watch resumption and status coalescing.
