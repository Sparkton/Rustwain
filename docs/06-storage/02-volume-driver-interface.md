# Volume Driver Interface

**K8s analogue:** CSI (Container Storage Interface) – Identity, Controller and Node gRPC
services; CSI sidecars (external-provisioner, external-attacher, external-resizer,
external-snapshotter, node-driver-registrar, livenessprobe).

## CSI recap

CSI defines three gRPC services a storage vendor implements:
- **Identity**: `GetPluginInfo`, `GetPluginCapabilities`, `Probe`.
- **Controller** (runs anywhere, talks to storage backend APIs): `CreateVolume`,
  `DeleteVolume`, `ControllerPublishVolume` (attach), `ControllerUnpublishVolume`,
  `CreateSnapshot`, `ControllerExpandVolume`, `GetCapacity`, `ListVolumes`, …
- **Node** (runs on every node, privileged): `NodeStageVolume` (format + mount to global
  staging path), `NodePublishVolume` (bind into pod path), `NodeUnpublish`,
  `NodeUnstage`, `NodeExpandVolume`, `NodeGetVolumeStats`, `NodeGetInfo` (topology).

In Kubernetes, a CSI driver deployment = the vendor's driver container + **4–6 sidecar
containers** (provisioner, attacher, resizer, snapshotter, registrar, liveness) that
translate Kubernetes objects into CSI calls. Each sidecar is a separate Go binary with its
own leader election, watches and memory footprint. A cluster with three CSI drivers
runs ~15–20 extra controller containers.

## Rustwain design

### Keep the CSI wire protocol, drop the sidecars `[proposed]`
- **Vendor drivers speak CSI** (the gRPC spec) – this gives us access to the entire
  existing ecosystem (EBS, PD, Azure Disk, Ceph RBD/CephFS, Longhorn, Portworx, NFS,
  OpenEBS, TopoLVM, …) **without forking them**.
- The **translation logic** that Kubernetes puts in sidecars lives in Rustwain's own
  controllers (volume controller, attach controller, snapshot controller) and the agent's
  volume manager, which call CSI endpoints directly.
- Driver registration: a `VolumeDriver` object (≈ CSIDriver) declares capabilities, plus the
  controller endpoint (a Service) and node endpoint (unix socket path on each node,
  discovered via a registration directory like kubelet's plugin registry
  `/var/lib/rustwain/plugins/<driver>/csi.sock`).

```
          ┌──────────── server (controllers) ─────────────┐
          │ volume ctl ──CSI Controller gRPC──▶ driver controller pod (vendor)
          │ attach ctl ──┘                                 │
          └────────────────────────────────────────────────┘
 node:    agent volume mgr ──CSI Node gRPC (unix sock)──▶ driver node pod (vendor, privileged)
```

Controllers calling out to a pod's gRPC endpoint: the server reaches the driver controller
via its Service IP – servers must have pod network access (they're agents too, or have a
route). `[open]`: or the driver controller **dials in** to the API (a reverse registration
like the agent tunnel). Lean: servers on the pod network; simplest.

### Native Rust drivers (in-tree, compiled in)
For storage we want to be excellent at without external pods:
- `local` – local disks/LVM/ZFS/btrfs with capacity tracking (see
  [local storage](03-local-storage.md)).
- `nfs` – mount NFS shares (provisioning via subdirectories).
- `hostpath-dev` – development only.

These implement the same internal `VolumeDriver` trait that the CSI client adapter
implements, so controllers don't care whether a driver is in-process or CSI.

```rust
#[async_trait]
pub trait ControllerDriver: Send + Sync {
    async fn create(&self, req: CreateDisk) -> Result<DiskInfo, DriverError>;
    async fn delete(&self, id: &DiskId) -> Result<(), DriverError>;
    async fn attach(&self, id: &DiskId, node: &NodeName, ro: bool) -> Result<AttachInfo, DriverError>;
    async fn detach(&self, id: &DiskId, node: &NodeName) -> Result<(), DriverError>;
    async fn expand(&self, id: &DiskId, size: Bytes) -> Result<ExpandInfo, DriverError>;
    async fn snapshot(&self, id: &DiskId, name: &str) -> Result<SnapshotInfo, DriverError>;
    async fn capacity(&self, topo: &Topology) -> Result<Option<Bytes>, DriverError>;
    fn capabilities(&self) -> ControllerCaps;
}

#[async_trait]
pub trait NodeDriver: Send + Sync {
    async fn stage(&self, req: StageRequest) -> Result<(), DriverError>;      // idempotent
    async fn publish(&self, req: PublishRequest) -> Result<(), DriverError>;  // idempotent
    async fn unpublish(&self, target: &Path) -> Result<(), DriverError>;
    async fn unstage(&self, staging: &Path) -> Result<(), DriverError>;
    async fn stats(&self, path: &Path) -> Result<VolumeStats, DriverError>;
    async fn expand(&self, req: NodeExpandRequest) -> Result<(), DriverError>;
    fn topology(&self) -> Topology;
}
```

### Idempotency and retries
CSI requires idempotent operations because orchestrators retry after timeouts. Our
controllers track operations with **operation records** (driver, op, volume, attempt,
deadline) so that:
- Retries reuse the same request IDs/names (CreateVolume `name` = Claim UID → idempotent).
- Concurrent operations on the same volume are serialised (CSI forbids concurrent ops on
  one volume; Kubernetes sidecars implement per-volume locks; drivers often return
  `ABORTED` otherwise).
- Long-running operations survive server restarts (state in the store).

### Mount propagation for node plugins
CSI node plugins run in privileged containers and mount into a host path that the agent
then bind-mounts into pods. Requires `Bidirectional` propagation for the driver's mount
dir (`/var/lib/rustwain/plugins/<driver>/mounts` → `rshared`) and `HostToContainer` for
pods consuming FUSE-backed volumes. The agent sets up the propagation-correct directory
structure at plugin registration.

### Userns and volumes
After NodePublish, the agent applies an **idmapped bind mount** onto the pod's volume
path using the pod's user namespace (see
[rootless and userns](../02-container-runtime/04-rootless-and-userns.md)), so drivers don't
need to know about userns at all. Filesystems that don't support idmap (NFS) fall back
per policy.

### fsGroup and permission changes
Kubernetes' `fsGroup` recursively chowns/chmods every file in the volume on every mount
(slow on large volumes; `fsGroupChangePolicy: OnRootMismatch` helps). With idmapped
mounts + consistent container-relative ownership on disk, recursive chown is unnecessary.
For drivers/volumes needing it, we use the driver's `VOLUME_MOUNT_GROUP` capability
(mount-option based group) or a one-time ownership fix recorded in volume metadata.

## Open questions

- Servers reaching CSI controller pods: pod network vs dial-in.
- Do we expose CSI-compatible node-side "ephemeral inline volumes"? (Used by secrets-store
  CSI driver) – yes via adapter.

## Build checklist

- [ ] `VolumeDriver` traits; CSI client adapter (tonic, CSI proto v1.x).
- [ ] Driver registration (VolumeDriver object + node plugin socket discovery).
- [ ] Operation records with per-volume serialisation.
- [ ] Run against the **CSI sanity test suite** (`csi-sanity`) with a mock driver – both as
      a client (our adapter) and for our native drivers wrapped as CSI servers.
- [ ] Integration test with an existing CSI driver (e.g. `csi-driver-host-path`,
      `csi-driver-nfs`).
