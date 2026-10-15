# Storage and etcd: Pros and Cons

## etcd

| Pros | Cons |
|---|---|
| Raft implementation is battle-tested (also used by many other systems) | Backend size quota (2 GiB default, ~8 GiB recommended max); exceeding → alarm, cluster read-only |
| MVCC + watch API fit Kubernetes perfectly | Compaction + defragmentation chores; defrag blocks member |
| Strong consistency, linearizable reads | Very sensitive to disk fsync latency → leader elections, timeouts |
| Simple data model | 1.5 MiB request limit constrains object sizes |
| Snapshot/restore tooling | Separate component: TLS, backups, upgrades, monitoring |
| | bbolt single-writer B+tree; write amplification; free pages not reclaimed without defrag |
| | Events and leases add heavy write churn (often split to separate etcd) |
| | Restores reset revisions → client cache confusion (mitigated by `--bump-revision`) |

### Alternatives in the ecosystem
| Project | Approach | Pros | Cons |
|---|---|---|---|
| kine (k3s) | etcd API shim over SQL (SQLite, Postgres, MySQL) | Use familiar databases; SQLite for single node | Watch implemented by polling; performance limits |
| embedded etcd (k3s HA) | etcd inside k3s binary | One binary | Same etcd limits |
| Google's internal (Spanner-backed for GKE large clusters) | Different backend | 65k-node clusters | Not available |
| FoundationDB-backed experiments | Distributed KV | Scale | Complexity |

## CSI

| Pros | Cons |
|---|---|
| Vendor-neutral, out-of-tree drivers; huge ecosystem | Kubernetes integration requires 4–6 sidecar containers per driver |
| Clear controller/node split | Idempotency and per-volume serialisation burdens on both sides |
| Snapshots, expansion, topology, capacity | Mount propagation requirements; privileged node plugins |
| Used by Nomad, Mesos too | Multi-attach/fencing safety relies on orchestrator behaviour (non-graceful shutdown taint is manual) |

## Persistent volume model

| Pros | Cons |
|---|---|
| PV/PVC split separates admin and user concerns | `Immediate` binding default causes zone mismatches |
| StorageClasses enable dynamic provisioning | RWO allows multiple pods on one node (surprise); RWOP came late |
| WaitForFirstConsumer topology-aware binding | fsGroup recursive chown slow on large volumes |
| Volume snapshots API | Local PVs awkward (static provisioning in-tree; node death = stuck) |
| | StatefulSet volumeClaimTemplates immutable (resizing via template not supported) |

## Storage systems commonly run on Kubernetes

| System | Pros | Cons |
|---|---|---|
| Rook/Ceph | Full-featured (block, file, object), scalable | Operationally heavy; resource hungry; expertise needed |
| Longhorn | Simple distributed block storage, UI, backups | Performance lower than Ceph/local; replicas per volume overhead |
| OpenEBS (Mayastor, LocalPV) | NVMe-oF engine (Mayastor), local options | Multiple engines, varied maturity |
| TopoLVM | Capacity-aware local LVM | Local-only |
| local-path-provisioner | Trivial | No quotas, no capacity awareness |
| Cloud disks (EBS/PD/Azure Disk) | Managed, durable | Zone-bound; attach limits per instance; attach/detach latency (seconds–minutes) |

## Takeaways for Rustwain
- Replace etcd with an embedded Raft + in-memory MVCC store that *is* the watch cache;
  bound size explicitly with quotas instead of hitting a cliff.
- Keep CSI on the wire; drop sidecars by putting the logic in our controllers.
- Safer defaults: `WaitForFirstConsumer`, `ExclusivePod` access, fencing-gated reattach.
- First-class local storage with capacity tracking and explicit node-loss policy.
- idmapped mounts replace fsGroup chowning.
