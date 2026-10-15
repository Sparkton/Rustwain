# Node Registration, Heartbeats and Health

**K8s analogue:** kubelet node registration, Node object, node leases (`kube-node-lease`),
NodeStatus updates, node lifecycle controller, taint-based eviction, node problem
detector, graceful node shutdown.

## Registration

1. Agent starts with a **bootstrap credential** (join token or cloud identity document;
   see [bootstrap and join](../10-cluster-operations/01-bootstrap-and-join.md)).
2. Generates a node keypair locally (private key never leaves the node; stored with 0600
   in `/var/lib/rustwain/pki/`, optionally in TPM `[later]`).
3. Submits a **certificate signing request** with the node identity (`node:<name>`).
4. Server-side approver validates (token scope, or cloud attestation) and signs.
5. Agent creates/updates its `Node` object: addresses, capacity, allocatable, features
   (from `HostFeatures`), isolation classes, device inventory, OS/kernel info, agent
   version.
6. Agent starts its lease loop and watches.

Node naming: hostname by default (lowercased, validated as DNS label); override via
config. Name collisions: the server rejects a second node claiming an existing name with
a different key unless the old node's certificate is revoked.

## Heartbeats: two channels

Kubernetes learned (KEP-589) that writing full NodeStatus every 10 s from every node
crushes etcd at scale, so it split:
- **Lease** object (tiny) renewed every 10 s → liveness.
- **NodeStatus** updated only on change, or every 5 min.

Rustwain keeps this split:
- `NodeLease { holder, renew_time, duration, agent_epoch }` renewed every 5 s (default),
  duration 20 s.
- **Leases are stored in a separate, non-persisted keyspace** `[proposed]`: lease renewals
  don't need durability across total cluster restart; they need replication only to make
  failover correct. Options:
  1. Persist through Raft like everything else (simple, but at 5,000 nodes × 1 write/5 s =
     1,000 writes/s of pure heartbeat traffic into the log).
  2. **Leader-local leases**: the Raft leader holds leases in memory; on leader change,
     the new leader grants a grace period equal to the lease duration before declaring any
     node dead (since it has no history). This is how many systems (e.g. Chubby-style
     designs) avoid durable heartbeats.
  Lean: option 2 – correctness preserved by the grace period, massive write reduction.
- `NodeStatus` written on change only (conditions, allocatable changes, images list
  changes are **not** written – Kubernetes' `node.status.images` list (up to 50 images)
  bloats Node objects; the scheduler's image locality scoring can use a separate compact
  summary).

## Clock independence

Lease expiry is evaluated by the server using its **own monotonic clock** from when it
last received a renewal – never by comparing the node's timestamp to the server's wall
clock. This means clock skew between nodes and servers cannot cause false node death
(a known Kubernetes edge case when `renewTime` is compared across machines).

## Node conditions

| Condition | Source | Effect |
|---|---|---|
| `Ready` | agent overall health (runtime ok, dataplane ok, can admit pods) | Scheduling; endpoints |
| `MemoryPressure` / `DiskPressure` / `PidPressure` | eviction manager | Taints; scheduling |
| `NetworkUnavailable` | dataplane | No pods scheduled |
| `KernelDeadlock`, `ReadonlyFilesystem`, `FrequentOOMs`, `NtpUnsynced`, `CorruptDocker…` | built-in problem detectors (kmsg parsing, fs checks) | Informational + optional taint |
| `ConfigPendingRestart` | config manager | Informational |
| `DeviceUnhealthy:<driver>` | device manager | Device not allocatable |

Built-in **problem detection** (Kubernetes' node-problem-detector is a separate
DaemonSet): read `/dev/kmsg` (structured, with sequence numbers – no need for journald)
and match patterns (hung task, ext4 errors, NVMe timeouts, MCE errors, OOM kills);
check filesystem read-only state; check time sync. Patterns are a config table.

## Node lifecycle controller (server side)

```
Lease fresh                → Ready (as reported)
Lease expired              → Unknown, taint `node.rustwain.io/unreachable:NoExecute`
                             pods with toleration window (default 60s stateless, 0 for
                             daemons, ∞ for StatefulSet-like until fenced) → marked for
                             replacement after window
Node reports Ready=false   → taint `not-ready:NoExecute` similarly
```

**Rate-limited eviction** across zones (Kubernetes' "zone disruption" logic): if a large
fraction of nodes in a zone go unreachable simultaneously, it's probably a network
partition of the control plane, not node failures – slow down or stop evictions to avoid
a mass reschedule storm. Keep this; it's a lesson learned in production.

## Network partitions: who is right?

A node partitioned from servers but not from its clients keeps serving (good for
availability). Its pods may be replaced elsewhere after the toleration window – now two
copies run. For stateless pods that's fine. For pods with exclusive volumes or singleton
semantics, replacements wait for **fencing**:
- The partitioned agent itself **self-fences** `[proposed]`: if it can't renew its lease
  for `lease_duration + safety_margin`, it stops pods that are marked `singleton` /
  have exclusive volumes. The server waits `lease_duration + safety_margin +
  max_clock_drift_rate × …` before rescheduling them. This is a lease-based fencing
  protocol – correct as long as clocks progress at bounded rates (monotonic clocks, not
  synchronised clocks).
- Plus optional hard fencing via out-of-band (IPMI/cloud API power-off) for paranoid
  setups.

See [HA and failure modes](../10-cluster-operations/03-ha-and-failure-modes.md).

## Graceful node shutdown

When the machine is shutting down (systemd `poweroff`, cloud preemption notice):
1. Agent holds a logind **delay inhibitor lock** while running.
2. On `PrepareForShutdown(true)` D-Bus signal: mark node `NotReady` / cordon, stop
   pods in priority order (lowest priority first, critical system pods last) within the
   configured budget, then release the lock.
3. Cloud preemption notices (spot instances) detected via cloud plugin → same path.
Kubernetes implements this (GracefulNodeShutdown + priority-based) – keep the design.
On non-systemd hosts: handle SIGTERM to the agent from the init system's shutdown
sequence (requires init to wait).

## Node removal
- `rwctl node drain <n>` → cordon + evict respecting disruption budgets (see
  [rollouts and disruption](../08-workloads/02-rollouts-and-disruption.md)).
- `rwctl node remove <n>` → revoke certificate, delete Node object, the agent (if alive)
  tears down everything and wipes its state.

## Open questions

- Leader-local leases vs persisted leases (needs careful proof of the failover grace logic).
- Default toleration windows per workload type.

## Build checklist

- [ ] CSR-based registration flow.
- [ ] Lease renewal loop + server-side lease table with monotonic expiry.
- [ ] kmsg problem detector with pattern table.
- [ ] Self-fencing logic with simulated partition tests.
- [ ] logind inhibitor integration (`zbus`) and priority-ordered shutdown.
