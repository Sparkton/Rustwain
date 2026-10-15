# Local Storage

**K8s analogue:** local PersistentVolumes (`local` volume type + static provisioner),
TopoLVM, OpenEBS LocalPV (hostpath, LVM, ZFS), Rancher local-path-provisioner,
ephemeral storage limits.

## Why local storage matters

NVMe SSDs give millions of IOPS and microsecond latency; network block storage gives
thousands of IOPS and milliseconds. Databases with their own replication (Cassandra,
CockroachDB, Kafka, Elasticsearch, Postgres with streaming replication) want local disks.
Kubernetes' local PV support is famously awkward: static provisioning only (in-tree),
no capacity awareness until CSI storage capacity tracking, node affinity hard-wiring that
makes pods unschedulable when the node dies.

## Rustwain `local` driver (native, in agent)

### Backends
| Backend | Provisioning | Snapshots | Quotas | Notes |
|---|---|---|---|---|
| **LVM thin** | Thin LV per Disk in a node VG | Thin snapshots (fast) | Size = LV size | Most general; needs `lvm2` userspace? → we can talk to device-mapper directly via ioctls (`DM_*`) but LVM metadata format is complex → `[open]` exec `lvcreate` vs native |
| **ZFS** | Dataset/zvol per Disk | Native, instant | refquota | Requires ZFS module (out-of-tree); exec `zfs`/libzfs_core via FFI |
| **btrfs** | Subvolume per Disk | Native snapshots | qgroups (slow at scale historically) | In-tree |
| **Directory + project quota** | Directory on xfs/ext4 | No (or reflink copy on xfs/btrfs) | Project quota (hard limit) | Simplest; good default for dev & small setups |
| **Raw device** | Whole disk/partition per Disk | No | n/a | Max performance |

Default `[proposed]`: **directory + project quota** (no extra setup) and **LVM thin**
(production). Backend chosen per StorageClass via `parameters.backend`.

### Capacity and scheduling
- Agent reports free capacity per backend pool in a `DeviceInventory`-like
  `StoragePool` status (per node) → scheduler filters nodes for unbound
  `WaitForFirstConsumer` claims and **reserves** capacity at binding time (avoids two pods
  racing for the last 100 GiB).
- Once bound, Disk is node-pinned → pod can only run on that node.

### Node failure with local volumes
The data is on the dead node. Options per workload (policy on the StorageClass/claim):
- `WaitForNode` (default): pod stays pending until node returns.
- `Abandon`: after a timeout, delete the claim's Disk record and let the workload create a
  fresh claim elsewhere (for replicated databases that rebuild from peers – StatefulSet
  controller integration: "replace replica with empty volume"). Kubernetes has no clean
  way to express this; operators hack it.

## Ephemeral storage

`scratch` volumes and container writable layers live on the node filesystem.

- **Accounting & enforcement**: project quotas per pod directory (O(1), enforced by
  kernel) – see [filesystems](../01-linux-foundations/04-filesystems.md).
- **Per-pod dedicated filesystem** option `[later]`: thin LV per pod for strong IO
  isolation (with `io.max` on that device).
- `medium: memory` scratch: tmpfs with `size=` = requested size, charged to pod memory.

## IO isolation
Noisy neighbours on a shared disk:
- `io.weight` / `io.cost` on the device for proportional sharing.
- `io.max` for hard caps (per-pod, per device).
- `io.latency` for latency-sensitive pods.
Requires knowing which device a pod's IO hits – for directory-backed volumes it's the
backing device of the filesystem (resolved via `st_dev` → `/sys/dev/block/maj:min`); for
LVM volumes, the dm device (cgroup IO control on dm devices applies to bios submitted to
dm; ensure the throttling layer sits where intended – generally configure on the dm device).

## Snapshots and backup of local volumes
- Snapshot → local (LVM/ZFS/btrfs) – fast, same node.
- **Export** snapshot to object storage `[later]`: stream the snapshot (with incremental
  diffs: `zfs send -i`, thin_delta for LVM, `btrfs send -p`) to S3-compatible storage →
  restore on another node. This is the backbone of app-level backup
  ([backup and DR](../10-cluster-operations/07-backup-and-disaster-recovery.md)).

## Open questions

- Native LVM metadata handling vs exec `lvm` tools (violates no-shell-out preference, but
  LVM's on-disk metadata format is complex; reimplementing is risky).
- ZFS support given licensing/out-of-tree module – support if present, don't require.

## Build checklist

- [ ] `local` driver with directory+quota backend.
- [ ] LVM thin backend (decision on exec vs native).
- [ ] StoragePool capacity reporting + scheduler reservation.
- [ ] `Abandon` node-failure policy + StatefulSet integration.
- [ ] IO isolation knobs per volume.
