# Image, Container and Disk Garbage Collection

**K8s analogue:** kubelet image GC (`imageGCHighThresholdPercent` 85 /
`imageGCLowThresholdPercent` 80, `imageMaximumGCAge`), container GC (dead containers),
containerd's GC (leases, content, snapshots), log rotation.

## What accumulates on a node

| Thing | Where | Grows with |
|---|---|---|
| Image blobs and unpacked snapshots | `/var/lib/rustwain/images` | Image churn (CI clusters: very fast) |
| Container writable layers | snapshot upper dirs | Containers writing to rootfs |
| Dead container records & their upper dirs | agent DB + disk | Restarts, completed jobs |
| Logs | `/var/log/rustwain/pods` | Chatty apps |
| emptyDir volumes | `/var/lib/rustwain/pods/<uid>/volumes` | Pod lifetime |
| Pod state dirs | `/run/rustwain/pods`, `/var/lib/rustwain/pods` | Leaks after crashes |
| Network state (BPF map entries, interfaces) | kernel | Leaks after crashes |
| UID ranges, IPs | allocator DB | Leaks after crashes |

## Principles

1. **Reference-counted roots, not heuristics**: everything reachable from a live pod
   (running or pending on this node) is never collected.
2. **Mark-and-sweep with intent logs**: every allocation writes an intent record first
   (see [low-level runtime](../02-container-runtime/02-oci-low-level-runtime.md)); GC sweeps
   anything not reachable from live roots *and* not under an active intent.
3. **Startup sweep**: on agent start, after recovering supervisors, sweep orphans
   (netns bind mounts, veths with `rw` prefix not attached to a live pod, cgroups under
   `rustwain.slice` without a supervisor, mounts under our dirs).
4. **Never block pod start on GC**, but **do** block admission if the disk is beyond
   the hard threshold (better to reject than to fill the disk).

## Image GC policy

Triggered by:
- Disk usage of the image filesystem above **high watermark** (default 85%) → collect
  until below **low watermark** (80%).
- Age: images unused for > `maxAge` (default 2 weeks `[proposed]`) are collected
  regardless (Kubernetes added `imageMaximumGCAge` in 1.29).
- Explicit: `rwctl node prune-images <node>`.

Ranking: least-recently-used (by last pod start referencing it), then by size
(free the most space first among equally old), never pinned images.

**Layer-level GC**, not image-level: a layer shared by 10 images stays as long as any
referencing image is kept. Collected unit = snapshot chain nodes with refcount 0.

### Pre-pulled and pinned images
- DaemonSet images and system images: pinned automatically while the DaemonSet targets
  the node.
- Users can pin via `ImagePrefetch` object `[proposed]` (pre-pull images to nodes matching
  a selector, keep pinned) – Kubernetes users do this with DaemonSets running `sleep`.

## Container GC

- Keep at most N dead instances per container (default 1 – for `logs --previous`) and
  their upper dirs only if `keepFailedFilesystem: true` (debugging) – otherwise upper dirs
  are deleted immediately on container exit/restart (frees disk faster than Kubernetes,
  which keeps dead containers around until GC).
- Completed pods (Job pods) on the node: local state removed once the final status is
  acknowledged by the API; the API object's lifetime is owner-controlled.

## Disk pressure interplay

```
usage ≥ image high watermark   → image GC
usage ≥ eviction soft threshold → image GC + dead container cleanup + log compaction,
                                  then evict pods exceeding ephemeral-storage requests
usage ≥ eviction hard threshold → reject new pods; evict top consumers immediately
```

Separate filesystems: if images live on a different filesystem than pod data
(`imagefs` vs `nodefs` in Kubernetes terms), thresholds apply per filesystem. Rustwain
detects the filesystem of each state directory with `statfs` + `st_dev`.

## Network and allocator GC

- IPAM: allocations keyed by pod UID; on startup, release any UID not in the live set
  (after a quarantine delay – IP reuse too soon can confuse conntrack/peers; default 30 s).
- BPF maps: endpoint entries for local pods not in the live set are removed; service maps
  are fully reconciled from API state.
- Interfaces: `rw*` host-side devices without a live pod are deleted.

## Open questions

- Should image GC consider *cluster-wide* information (e.g. "this image is about to be
  used by a rollout") to avoid evicting an image right before it's needed? A rollout
  controller could pre-pin. `[open]`.

## Build checklist

- [ ] Refcounted snapshot chain GC with watermarks and age limit.
- [ ] Startup orphan sweeper for every resource type, tested with crash injection.
- [ ] Per-filesystem threshold tracking.
- [ ] `ImagePrefetch` object + agent support.
