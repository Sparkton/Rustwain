# Heterogeneous Fleets

**K8s analogue:** mixed-architecture clusters (multi-arch images + `kubernetes.io/arch`
labels), mixed OS/kernel nodes, Node Feature Discovery, edge (KubeEdge, k3s, OpenYurt),
GPU node pools, spot/on-demand mixes.

This document is the operational counterpart of
[Linux host variance](../01-linux-foundations/05-linux-host-variance.md): given machines
that differ, how does the *cluster* behave sensibly?

## Sources of heterogeneity
- CPU arch (x86_64, aarch64), CPU features (AVX-512, AMX, SVE), core types.
- Page size (4K/16K/64K).
- Kernel versions and features → dataplane backend (eBPF vs nftables), isolation classes
  available, idmapped mount support.
- Distros and LSMs.
- Hardware: GPUs, NICs (SR-IOV, RDMA), local NVMe, memory size.
- Network location: datacenter vs edge (high latency, intermittent connectivity, NAT).
- Purchase model: on-demand, spot, reserved.

## Mechanisms

### 1. Attributes everywhere
Every difference is a typed node **attribute** (see [placement](../08-workloads/06-placement.md)).
Workloads declare requirements; the scheduler matches. Images declare platform
(arch/variant/page size/CPU level) – the scheduler can **auto-derive** placement
requirements from the image's available platforms (`[proposed]`: if an image index has
only `linux/amd64`, the pod is automatically restricted to amd64 nodes – Kubernetes
doesn't do this; pods land on arm64 nodes and fail with `exec format error`).

### 2. Mixed dataplane backends in one cluster
Nodes running the eBPF dataplane and nodes running the nftables fallback must
interoperate:
- Same addressing, routing modes, encapsulation formats (Geneve with identity option –
  nftables nodes can't read the option → they fall back to ipcache-by-IP for identity; eBPF
  nodes must handle packets **without** the option from nftables nodes).
- Policy semantics identical (conformance tests run on both backends).
- Service LB: socket-LB on eBPF nodes vs DNAT on nftables nodes – client-visible behaviour
  identical (except conntrack timeouts).

### 3. Feature-level gating
Cluster features that require all nodes to support something (e.g. WireGuard encryption
mandatory, IPv6-only) are enabled only when **all** nodes report support; `rwctl cluster
features` shows blockers ("3 nodes lack WireGuard module").

### 4. Mixed isolation classes
`microvm` workloads only on KVM-capable nodes; tenants requiring microvm get a
`NoSchedule` toleration-like matching automatically.

### 5. Edge nodes `[later]`
- Agents behind NAT: dial-out tunnel (already the design).
- Intermittent connectivity: longer lease durations per node pool (e.g. 5 min), with
  workloads tolerating unreachable for hours; static stability means they keep running.
- Bandwidth-limited links: image pulls via local registry mirror per site; delta watches.
- Small hardware (Raspberry Pi 4/5, 2–8 GB): agent footprint < 40 MB matters here.
- Node pools with `edge: true` excluded from cluster-wide features requiring low latency
  (e.g. full-mesh WireGuard: edge nodes use hub-and-spoke through gateway nodes).

### 6. Spot capacity
- Node attribute `capacity.type=spot`; interruption handling via cloud plugin; workloads
  opt in via placement preferences; autoscaler diversifies instance types.

## Multi-arch images in practice
- Build: `docker buildx`/`buildah` produce image indexes; Rustwain doesn't build images.
- Pull: platform selection with variant fallbacks (`arm64` → `v8`).
- Mirrors: ensure mirrors keep full indexes (some mirroring tools strip non-amd64 manifests
  → arm64 nodes fail). `rwctl image inspect` shows available platforms.

## Testing
- CI matrix includes an arm64 runner and mixed clusters (amd64 servers + arm64 agents).
- Interop tests between dataplane backends.

## Open questions

- Auto-derived placement from image platforms: needs image index resolution at admission
  (image resolver controller) – acceptable latency?
- Hub-and-spoke WireGuard topology for edge pools.

## Build checklist

- [ ] Image-platform-derived scheduling constraints.
- [ ] Cross-backend dataplane interop tests.
- [ ] Cluster feature gating on node support.
- [ ] Per-pool lease duration settings.
