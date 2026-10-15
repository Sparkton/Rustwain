# Startup Latency

**K8s analogue:** pod startup SLO (5 s p99 for stateless pods with pre-pulled images in
the official scalability SLOs), image pull time, sandbox creation, CNI setup, kubelet
sync loops, scheduler latency.

Every millisecond between "user asks for a pod" and "process is running" is accounted for
here. The pod status `timings` field makes these phases visible per pod.

## Anatomy of a pod start (Kubernetes, typical warm-image numbers)

| Phase | Typical | Why |
|---|---|---|
| API write + admission | 10–50 ms | webhooks can add 100s of ms |
| Controller creates pod (Deployment → RS → Pod) | 10–100 ms | two controller hops, informer delays |
| Scheduler | 5–50 ms | queueing, filters |
| Binding → kubelet sees pod | 10–100 ms | watch delivery |
| kubelet sync, volume mounts | 50–500 ms | volume manager polling loops (historically 100ms–1s loops) |
| Sandbox: pause container + CNI | 100–700 ms | runc start of pause, CNI plugin chain exec |
| Image pull (warm: check only) | 5–50 ms | |
| Container create/start (runc) | 50–200 ms | shim, runc init re-exec |
| **Total** | **~0.5–2 s** | |

## Rustwain targets and how each phase is shortened

| Phase | Target | Technique |
|---|---|---|
| API write + admission | 3–5 ms | in-process CEL policies, no webhooks by default, group commit |
| Workload controller → pod | 5–10 ms | direct pod management from Deployment (no RS hop), shared in-process watch |
| Scheduler | 1–2 ms | in-process watch, cached equivalence, batch placement |
| Binding → agent | 2–5 ms | index-dispatched watch, encode-once |
| Agent admission + prepare (parallel) | 5–20 ms | parallel images/volumes/IP/devices; local IPAM; no polling loops |
| Sandbox (namespaces, netns wiring) | 5–10 ms | no pause container; netlink setup < 5 ms |
| Container start | 3–10 ms | supervisor clone3 + exec; precompiled seccomp; new mount API |
| **Total (warm, process class)** | **~50–100 ms (p50)** | |

## Cold starts: images dominate
For a 500 MB image over a 1 Gbit/s link, the pull alone is ≥ 4 s plus decompression and
unpacking. Techniques (see [images](../02-container-runtime/03-images-and-snapshotters.md)):
1. Parallel layer download + streaming decompression + parallel unpack.
2. zstd layers.
3. Pre-pulling (ImagePrefetch) for known-upcoming rollouts: the rollout controller can
   pre-pull the new revision's image to nodes *before* replacing pods
   (`prefetch: true` on Deployment strategy `[proposed]`) – rollouts then don't pay pull
   time per batch.
4. Lazy pulling (EROFS on-demand) `[later]`.
5. P2P distribution within the cluster `[open]`.

## MicroVM starts
- Snapshot/restore templates (see [KVM and microVMs](../03-virtualization/02-kvm-and-microvms.md)).
- Pre-warmed pool of idle VMs per shape per node (`[open]`: costs memory; maybe 1–2 per
  node for latency-sensitive tenants).

## Application-side startup
Out of our control, but we help:
- `startupProbe` with fast initial checks (period 1 s) so readiness is detected quickly.
- Readiness detected → endpoints updated in < 200 ms cluster-wide.
- CPU boost at startup `[proposed]`: temporarily raise `cpu.max`/weight for the first N
  seconds (JVM/JIT-heavy apps start much faster; Kubernetes needs in-place resize hacks
  like Google's "startup CPU boost" project). With in-place cgroup writes this is trivial.

## Measuring
- `PodTimings` in status (phase timestamps from agent; API/scheduler timestamps in
  metadata).
- `rwctl debug pod-timing <pod>` – waterfall view.
- CI bench: 100 pods created simultaneously on one node; p50/p99 to Running.

## Build checklist
- [ ] `PodTimings` plumbing across components.
- [ ] Waterfall CLI view.
- [ ] Rollout prefetch.
- [ ] Startup CPU boost option.
- [ ] Bench: pod start latency suite (warm, cold, microvm).
