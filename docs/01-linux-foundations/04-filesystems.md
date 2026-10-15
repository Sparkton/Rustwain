# Filesystems for Containers

**K8s analogue:** containerd snapshotters (overlayfs, native, devmapper, stargz),
runc rootfs setup, kubelet volume mounting, ephemeral storage accounting.

## What a container's filesystem view is made of

```
/                       ← overlayfs: image layers (read-only lowerdirs) + per-container upperdir
├─ proc                 ← procfs (new instance, in pod PID ns)
├─ sys                  ← sysfs (ro)
├─ dev                  ← tmpfs with a minimal set of device nodes (bind-mounted from host)
│  ├─ pts               ← devpts (new instance, newinstance,ptmxmode=0666)
│  ├─ shm               ← tmpfs (pod-shared via ipc ns, size-limited)
│  └─ mqueue            ← mqueue fs
├─ etc/hosts            ← bind mount, generated per pod
├─ etc/hostname         ← bind mount
├─ etc/resolv.conf      ← bind mount, generated per pod
├─ run/secrets/...      ← tmpfs, projected service identity / secrets
└─ data                 ← volume mount (bind from host dir / block dev / network fs)
```

## overlayfs

### How it works
- `lowerdir=L3:L2:L1` (read-only, leftmost is top), `upperdir=U` (writable),
  `workdir=W` (same fs as upper, scratch for atomic ops), mounted at `merged`.
- Reads fall through layers top-down. Writes **copy-up** the whole file from lower to
  upper before modifying (so editing a 1 GB file copies 1 GB). Deletes create
  **whiteouts** (char device 0/0) in upper; deleted directories get an opaque xattr.
- `metacopy=on` (4.19+): chmod/chown copy only metadata, not data – big win for
  user-namespace setups that touch ownership. Has security caveats with untrusted
  lowers; only with trusted layers.
- `redirect_dir=on`: renames of directories don't copy the whole tree.
- `index=on`, `nfs_export=on`: hardlink consistency, NFS export – not needed.
- `volatile` (5.10+): skip `sync` on upper – containers' writable layers are
  disposable, so this is a meaningful speedup for write-heavy containers. Must discard
  the upper dir after a host crash (the kernel marks it). Rustwain uses `volatile` for
  container upper dirs by default.
- Max lowerdirs: limited by mount option string page size (~500 layers at short paths).
  Use short paths (`/var/lib/rustwain/l/<shortid>` symlinks, like containerd/Docker do)
  or the new mount API's `fsconfig(FSCONFIG_SET_STRING, "lowerdir+", …)` (6.8+) which
  appends one layer per call without a length limit.

### Backing filesystem requirements
| Backing fs | Works for upper? | Notes |
|---|---|---|
| ext4 | yes | Most common |
| xfs | yes **only if `ftype=1`** | Old RHEL 7 installs may have ftype=0 – preflight must check (`xfs_info`, or `statfs` + `d_type` test by creating a file and reading `d_type` via `getdents`) |
| btrfs | yes | Also has native snapshot driver option |
| zfs | 2.2+ supports overlay upper | Older needs zfs driver |
| tmpfs | yes | Diskless nodes |
| overlayfs | no (nested overlay as upper not supported) | Rootless-in-container scenarios |
| NFS | lower only | |

### Unprivileged overlay
Since 5.11, overlayfs can be mounted inside a user namespace (with restrictions). This
lets rootless mode use overlay instead of slow `fuse-overlayfs`.

## Snapshot drivers

Rustwain's image store (see [images and snapshotters](../02-container-runtime/03-images-and-snapshotters.md))
supports pluggable snapshot drivers:

| Driver | How | Pros | Cons |
|---|---|---|---|
| **overlay** (default) | Each layer unpacked to a dir; container = overlay mount | Fast, universal, page cache shared across containers | Copy-up cost, inode use |
| **btrfs** | Each layer a subvolume snapshot of its parent | Instant snapshots, CoW at block level | btrfs only, metadata overhead |
| **zfs** | Datasets + clones | Same, with ZFS features | ZFS only, licensing (out-of-tree module) |
| **erofs + fscache / composefs** | Layers as read-only EROFS images, content-addressed file dedup via fs-verity | Integrity verification, dedup, lazy loading | Newer kernels (6.x), more moving parts |
| **block (devmapper / lvm thin)** | Each layer a thin snapshot | Needed for VM-backed pods that want a block device | Slower to create, complex |
| **lazy (stargz/SOCI-like)** | Mount via FUSE, fetch chunks on demand | Start before full pull | FUSE overhead, network dependence |

Decision `[proposed]`: ship `overlay` first, design the trait to allow `erofs`/
`composefs` and `block` next (block is required for microVM rootfs; see
[KVM and microVMs](../03-virtualization/02-kvm-and-microvms.md)).

### composefs: worth watching
composefs = an EROFS metadata image + overlayfs with a content-addressed object store as
data-only lower layer + fs-verity digests. Gives: per-file dedup across all images on
the node (identical files stored once), tamper detection, page-cache sharing across
images. Requires 6.6+ for the full feature set. This is the most interesting direction
for density and integrity – plan to prototype after milestone 2.

## Idmapped mounts

The problem: with user namespaces, container UID 0 = host UID 100000 (say). Image files
are owned by host UID 0. Without help, the container sees its files as owned by
`nobody` and can't write them.

Old fix: `chown -R` the unpacked image per userns mapping (slow, wastes disk, kills
sharing). New fix: **idmapped mounts** – `mount_setattr(fd, "", AT_EMPTY_PATH,
MOUNT_ATTR_IDMAP, userns_fd)` creates a mount where file ownership is translated through
the userns mapping. Zero copy, layers shared across all pods regardless of mapping.

Filesystem support (approximate, check per kernel): ext4/xfs/btrfs/fat 5.12+,
overlayfs 5.19+ (idmapped lower layers), tmpfs 6.3+, cephfs 6.7+, NFS no. The host
probe tests idmap support per filesystem we actually use by trying it on a scratch dir.

Fallback when unsupported: per-mapping chowned copy (cached per mapping), or disable
userns for that pod (policy decision).

## Mount propagation

| Mode | Behaviour | Use |
|---|---|---|
| private | No propagation either way | Default for container mounts |
| rslave | Host → container | Volumes where the host may mount later (FUSE, CSI) |
| rshared | Both ways | Privileged storage drivers mounting for other pods |
| unbindable | Can't be bind-mounted | Rarely |

Kubernetes `mountPropagation: None | HostToContainer | Bidirectional` maps directly.

## Device nodes

Minimal `/dev`: `null, zero, full, random, urandom, tty, console (pty), ptmx → pts/ptmx,
fd → /proc/self/fd, stdin/out/err` symlinks, `shm`, `mqueue`. Created by bind-mounting
host device nodes (works under userns; `mknod` doesn't), with cgroup device eBPF
allowing exactly these plus requested devices.

## /etc/hosts, /etc/resolv.conf, /etc/hostname

Generated per pod in `/run/rustwain/pods/<uid>/etc/` and bind-mounted read-only (or
read-write copy if `allowEtcWrites`). resolv.conf content is covered in
[DNS](../05-networking/05-dns-and-discovery.md). Important: bind-mounting a single file
pins the inode – tools that "edit" by rename (sed -i) break the bind. We document this
and offer writable copies.

## Ephemeral storage accounting

Kubernetes measures container writable layer + emptyDir + logs usage by walking
directories with `du`-like scans every few seconds – expensive on busy nodes with many
small files.

Rustwain options:

1. **Project quotas** (xfs, ext4 with `prjquota`): assign a project ID per pod's
   directories, kernel tracks usage in O(1) and can **enforce** limits. Kubernetes has
   `LocalStorageCapacityIsolationFSQuotaMonitoring` (beta for years) for this.
2. **Per-pod filesystem** (loopback image, LVM thin volume, btrfs subvolume with qgroups)
   – strong isolation, more setup cost.
3. **Periodic scan** – fallback.

Decision `[proposed]`: project quotas when the backing fs supports them (detected),
scan fallback otherwise. Preflight warns when `/var/lib/rustwain` is on a filesystem
without project quota support.

## tmpfs and memory accounting

`emptyDir: { medium: Memory }` and `/dev/shm` are tmpfs. tmpfs pages are charged to the
memory cgroup of the process that first touched them, and count as `shmem` – not
reclaimable without swap. Set `size=` on every tmpfs (Kubernetes historically didn't for
`/dev/shm` → 64 MB Docker default, or unbounded). Rustwain sizes shm tmpfs to the pod's
memory limit (or explicit size) so usage is visible and bounded.

## fs-verity and integrity

`fs-verity` (ext4/f2fs 5.4+, btrfs 5.15+) gives per-file Merkle tree verification,
enforced on read. Combined with signed images, a node can refuse to execute tampered
layer files. Part of the composefs direction; see
[supply chain](../09-security/04-supply-chain.md).

## Rust implementation notes

- New mount API via `rustix::mount::{fsopen, fsconfig_*, fsmount, move_mount, open_tree,
  mount_setattr}`.
- Path safety: all operations inside the rootfs use `openat2` with
  `RESOLVE_IN_ROOT`/`RESOLVE_NO_MAGICLINKS` – a malicious image can contain symlinks like
  `/etc -> /host/etc` that cause mounts to land on the host. This class of bug has hit
  runc several times (CVE-2021-30465 symlink-exchange race on mount targets). The
  `pathrs` / `libpathrs` (Rust, by runc maintainer) crate exists exactly for safe path
  resolution – strong candidate.
- Tar unpacking of image layers: `tar` crate + our own whiteout handling; sandboxed with
  Landlock + seccomp and run in a separate process because it parses untrusted input.

## Open questions

- Default snapshotter for VM pods: block devices (devmapper thin / qcow2-like) vs
  virtio-fs sharing an overlay dir. See KVM doc.
- composefs as a default once kernel floor allows.

## Build checklist

- [ ] Overlay mount builder using new mount API with `lowerdir+` fallback.
- [ ] Preflight: xfs ftype, d_type, project quota support, idmap support per fs.
- [ ] Safe path resolution module (evaluate `pathrs`).
- [ ] Rootfs assembly test: symlink-attack image fixtures must not escape.
- [ ] Project-quota ephemeral storage accounting with fallback scanner.
