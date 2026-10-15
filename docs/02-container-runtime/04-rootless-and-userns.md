# User Namespaces by Default, and Rootless Nodes

**K8s analogue:** `hostUsers: false` (KEP-127, beta in 1.30, on by default for the
feature gate in 1.33 but still opt-in per pod), rootless kubelet (usernetes, k3s rootless).

Two distinct things:
1. **User-namespaced pods** – the agent runs as root, but every pod gets its own UID
   range so container root ≠ host root.
2. **Rootless nodes** – the agent itself runs as an unprivileged user.

## 1. User-namespaced pods (default on)

### Why
Without user namespaces, UID 0 in a container *is* UID 0 on the host, restricted only
by capabilities/seccomp/LSM. Any kernel or runtime bug that lets a process escape the
mount namespace yields host root. With user namespaces, an escape yields an unprivileged
UID that owns nothing on the host. Many historical container escapes (including
CVE-2019-5736 and CVE-2024-21626) would have been neutralised or greatly reduced.

### ID allocation
- Each pod gets a **contiguous range of 65,536 IDs** on the host
  (container 0..65535 → host base..base+65535).
- Ranges come from a node-local allocator over a configured host range (default
  `[1_000_000_000, 2^32 − 1)` → ~50,000 pods worth of ranges, more than enough for any node
  lifetime with reuse).
- Ranges **are not reused until all files owned by that range are gone** (writable layers
  and emptyDirs deleted) – prevents a new pod inheriting old files' ownership.
- Persistent volumes complicate this – see below.
- Kubernetes uses 65536-ID ranges and a fixed-ish allocation within kubelet; we do the
  same but make it explicit in the pod status (`hostUidBase: 1000065536`) for debugging.

### Making images work: idmapped mounts
Image files are owned by container-relative IDs (0 = root). Overlay lower layers are
shared across pods with *different* mappings. Solutions in order of preference:

1. **Idmapped mounts** (see [filesystems](../01-linux-foundations/04-filesystems.md)): mount
   each layer (or the overlay) with the pod's userns mapping. Zero-copy, shared page
   cache. Kernel 5.19+ for overlay; underlying fs must support idmap.
2. **Chowned copy per mapping** – unpack a second copy with shifted ownership. Slow,
   disk-heavy, but works everywhere.
3. **Disable userns for this pod** – only if policy allows (`hostUsers: true` requires a
   privileged-ish security level).

### Volumes and userns
| Volume | Approach |
|---|---|
| emptyDir, secrets, config, downward API | Created by agent, chowned to the pod's mapped IDs (cheap; they're new) |
| Persistent volumes (exclusive to one pod at a time) | **Idmapped mount** of the volume with the pod's mapping, so on-disk ownership stays container-relative (0..65535) regardless of which host range the pod gets next time. This is the key trick that makes PVs portable across pods/nodes |
| Shared filesystems (NFS) | NFS doesn't support idmapped mounts (as of 6.x). Options: run pod without userns, or rely on NFS server-side squashing. Document |
| hostPath | Owned by host IDs; container sees `nobody` unless idmapped; mostly used by privileged system pods which run without userns |

### Capabilities under userns
- Capabilities apply only to resources owned by the userns. `CAP_NET_ADMIN` in the pod
  userns lets the pod configure *its own* netns (owned by its userns) but nothing on the
  host. `CAP_SYS_ADMIN` lets it mount some filesystem types (tmpfs, proc, sysfs (with
  netns), overlay (5.11+), fuse (4.18+)) – meaning user-namespaced pods can do things like
  run nested containers (buildkit, podman) **without privileged mode**. A big usability
  win: "Docker-in-Docker" for CI without `privileged: true`.
- Kernel attack surface grows: unprivileged userns exposes code paths historically
  rich in bugs (netfilter, overlay, etc. reachable by "root in userns"). Our default
  seccomp profile should **block `unshare(CLONE_NEWUSER)` and `clone` with
  CLONE_NEWUSER inside containers** unless the workload opts in (`allowNestedUserns`).
  This keeps the benefit (pod root ≠ host root) without granting nested userns creation
  by default.

### Compatibility gotchas
- Files in images owned by UIDs > 65535 (rare; some enterprise images use large UIDs)
  → map larger ranges for those pods (policy: up to 2^24) or fail with a clear error.
- `setuid` binaries work within the mapping.
- Some software checks `/proc/self/uid_map` and behaves differently (e.g. systemd in
  containers, which actually works better with userns).
- Kernel keyring, `CAP_SYS_TIME`, `CAP_SYS_MODULE`, loading BPF programs (needs
  `CAP_BPF` in init userns) – not available. Correct for almost all workloads.
- Performance: idmapped mounts add small per-inode overhead on permission checks –
  measured as negligible in published benchmarks; verify in our perf suite.

## 2. Rootless nodes

Running the entire agent as an unprivileged user: useful for development laptops,
shared HPC machines, CI runners, and as a hardening option for edge devices.

### What changes
| Concern | Rootful | Rootless |
|---|---|---|
| cgroups | Own `rustwain.slice` | Need systemd **user** delegation: `systemctl --user` with `Delegate=cpu cpuset io memory pids` (requires `/etc/systemd/system/user@.service.d/delegate.conf`) |
| UID ranges | Allocate freely | Limited to `/etc/subuid`/`/etc/subgid` ranges for the user (typically 65536 IDs total → only one pod with full range, or many with small ranges) → **must use `newuidmap`/`newgidmap` setuid helpers** (shadow-utils) to write maps |
| Networking | Real netns + eBPF on host | No host netns control: use **slirp4netns**/**pasta** (passt) user-mode networking or **bypass4netns**; or a "rootlesskit" outer netns with a TAP. eBPF dataplane unavailable (no CAP_BPF in init ns) |
| Overlay | Kernel overlay | Kernel overlay in userns (5.11+) – OK |
| Privileged ports | Any | Need `net.ipv4.ip_unprivileged_port_start` or port forwarding via the network helper |
| Devices, KVM | Any | Only if the user is in `kvm` group etc. |
| Cluster networking | Full | Limited: cross-node pod networking needs a userspace overlay (e.g. VXLAN in the rootless netns via rootlesskit) |

Decision `[proposed]`: rootless is a **separate mode** (`rustwain agent --rootless`) with
a reduced feature set, targeted at dev and CI. Networking uses `pasta` (fast, maintained
by the Podman/passt project) via exec, or our own Rust TAP-based user-mode stack
`[open]` (large project – smoltcp could be a base). Milestone: after the main path is solid.

## Open questions

- Should we offer per-pod *smaller* ranges (e.g. 4,096 IDs) for density in rootless
  mode? Images rarely use IDs above 4,096 except `nobody` (65534) – which breaks with a
  small range. Could map 65534 specially (non-contiguous maps support up to 340 extents
  since 4.15).
- Default for blocking nested userns – measure how many common workloads need it (buildkit,
  some browsers' sandboxes – Chrome uses userns for its sandbox!). Chrome-in-container
  (Playwright, Puppeteer) is common → may need a well-known opt-in.

## Build checklist

- [ ] Host ID range allocator with "not reused until files gone" tracking.
- [ ] Idmapped mount support detection and per-fs fallback strategy.
- [ ] PV idmapping on attach.
- [ ] Seccomp default denying nested userns with an opt-in flag.
- [ ] Rootless mode spike: cgroup user delegation + newuidmap + pasta.
