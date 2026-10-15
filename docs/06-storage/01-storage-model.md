# Storage Model

**K8s analogue:** Volumes (emptyDir, hostPath, configMap, secret, projected, downwardAPI,
ephemeral, image), PersistentVolume, PersistentVolumeClaim, StorageClass, access modes,
volume binding modes, attach/detach controller, volume expansion, storage capacity
tracking, VolumeAttachment.

## Volume kinds

| Kind | Lifetime | Backing | Notes |
|---|---|---|---|
| `scratch` (emptyDir) | Pod | Node disk or tmpfs (`medium: memory`) | Size limit enforced via project quota |
| `config` / `secret` / `downward` / `projected` | Pod | tmpfs, materialised by agent | Atomic updates via symlink swap; see [config delivery](../08-workloads/05-config-and-secret-delivery.md) |
| `image` | Pod | Read-only OCI snapshot | Models, static assets |
| `host` (hostPath) | Node | Host directory | Privileged security level only |
| `claim` | Independent | Disk via driver | Persistent |
| `ephemeral-claim` | Pod | Disk provisioned per pod, deleted with pod | Big scratch on network/local storage |

## Persistent storage objects

```
StorageClass  (admin: "how to provision": driver, parameters, reclaim, binding mode, expansion)
      ▲
Claim         (user: "I need 50Gi RWO fast storage")  ── bound to ──▶  Disk (provisioned volume)
      ▲                                                                   │
Pod.volumes.claim                                                  Attachment (Disk ↔ Node)
```

- `Claim` ≈ PVC, `Disk` ≈ PV, `Attachment` ≈ VolumeAttachment.
- **Access modes** (kept, clarified):
  - `ExclusivePod` (ReadWriteOncePod) – one pod. **Default** `[proposed]` – what people
    usually mean when they write RWO, and it prevents two pods on the same node sharing a
    volume unexpectedly (a real data-corruption footgun with RWO).
  - `ExclusiveNode` (ReadWriteOnce) – one node, many pods.
  - `SharedReadOnly` (ReadOnlyMany).
  - `Shared` (ReadWriteMany) – network filesystems.
- **Volume mode**: `Filesystem` or `Block` (raw device for databases and VMs).
- **Binding**: `WaitForFirstConsumer` default `[proposed]` (provision in the zone/node
  where the pod is scheduled; Kubernetes defaults to `Immediate`, which causes zone
  mismatches – pods unschedulable because the volume landed in another zone).
- **Reclaim**: `Delete` (default for dynamic) or `Retain`.

## The lifecycle of a claim-backed volume

```
1. User creates Claim (storageClass: fast)
2. Pod referencing the Claim is created
3. Scheduler: considers volume topology (Disk's accessible nodes/zones, or for unbound
   WaitForFirstConsumer claims, the class's allowed topologies + capacity) → picks node
4. Volume controller: provision Disk via driver (CreateVolume, in the node's zone) → bind
5. Attach controller: Attachment(Disk, node) → driver ControllerPublish (cloud attach API)
6. Agent: wait for Attachment ready → driver NodeStage (format if new, mount to global path)
         → NodePublish (bind mount into pod's volume path, idmap if userns)
7. Pod runs
8. Pod deleted → agent NodeUnpublish → (last user on node) NodeUnstage → Attachment deleted
   → driver ControllerUnpublish (detach)
9. Claim deleted → reclaim policy → driver DeleteVolume
```

These are CSI's phases; the interface is described in
[volume driver interface](02-volume-driver-interface.md).

## Safety: never double-attach exclusive volumes

The worst storage bug class: two nodes writing the same block volume (filesystem
corruption). Happens in Kubernetes when a node is partitioned, the pod is force-deleted,
and the volume is attached elsewhere while the old node still has it mounted. Kubernetes
added "non-graceful node shutdown" (`node.kubernetes.io/out-of-service` taint that an
admin applies after confirming the node is down).

Rustwain:
- Detach of an exclusive volume from an unreachable node requires **fencing** (see
  [registration](../04-node-agent/05-node-registration-and-heartbeat.md)): lease expired
  + self-fence window elapsed, or hard fence (power off) confirmed, or admin override.
- Drivers can provide **storage-level fencing** (SCSI persistent reservations, cloud
  API "force detach", Ceph blocklisting of the old client) – used when available; the
  Attachment records the fence method used.
- Status shows `WaitingForFence{node, since, method}` clearly instead of a pod stuck in
  `ContainerCreating` with `Multi-Attach error` events.

## Expansion
- Claims can grow (if class allows): controller expands Disk via driver; node-side
  filesystem resize (`resize2fs`/`xfs_growfs` – via ioctls `EXT4_IOC_RESIZE_FS`,
  `XFS_IOC_FSGROWFSDATA` directly, no tool exec) while mounted where supported.
- Shrinking: not supported (same as Kubernetes; few filesystems support it).

## Capacity tracking
- Drivers report capacity per topology segment (node for local storage, zone for cloud)
  → scheduler avoids nodes/zones without capacity for `WaitForFirstConsumer` claims
  (Kubernetes `CSIStorageCapacity`). Essential for local storage.

## Snapshots and clones
- `Snapshot` (≈ VolumeSnapshot) / `SnapshotClass`; `Claim.spec.source: { snapshot | claim }`
  for restore/clone; **volume populators** for arbitrary sources (HTTP image, registry,
  backups).
- Group snapshots (crash-consistent across multiple volumes of one app) `[later]`.

## Volume health `[later]`
Drivers report abnormal conditions (disk degraded, mount stale) → events + pod
conditions.

## Open questions

- Default access mode `ExclusivePod` – compatibility impact for imported Kubernetes
  manifests using RWO with multiple pods on one node (rare but real: e.g. log sidecar pod).
- Should Claims be namespaced and Disks cluster-scoped (as in k8s)? Yes – keeps admin vs
  user separation.

## Build checklist

- [ ] Types: StorageClass, Claim, Disk, Attachment, Snapshot, SnapshotClass.
- [ ] Volume controller (provision/bind/expand/delete) with WaitForFirstConsumer.
- [ ] Attach controller with fencing gates.
- [ ] Scheduler volume topology + capacity filters.
- [ ] Agent volume manager: stage/publish/unpublish/unstage state machine, checkpointed.
