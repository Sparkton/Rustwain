# Kubernetes: Pros and Cons, Component by Component

An honest assessment. Kubernetes is the most successful infrastructure project of its
generation; most of its "cons" are consequences of reasonable decisions made under real
constraints (backwards compatibility, many vendors, a 2014 starting point). The point of
this document is to know exactly what to keep and what to change. Each con links to the
[pain point tracker](06-pain-point-tracker.md) ID where Rustwain addresses it.

## Overall

### Pros
- **Declarative, level-triggered reconciliation** – the core idea; robust to failures and
  restarts. *Keep.*
- **Uniform API** for everything, with labels/selectors and spec/status split. *Keep.*
- **Extensibility** (CRDs + controllers) – created an ecosystem. *Keep, improve.*
- **Portability** across clouds and on-prem. *Keep.*
- **Huge ecosystem and talent pool.** Can't replicate; must interoperate (OCI, CSI,
  Gateway API shapes, import of manifests).
- **Mature failure handling** learned from Borg/Omega.
- **Open governance**, vendor neutrality.

### Cons
- **Operational complexity**: many components, many config surfaces, many CAs/certs.
  (PP-01)
- **Steep learning curve**: dozens of object kinds before a "hello world" is production
  ready (Deployment, Service, Ingress, ConfigMap, Secret, HPA, PDB, NetworkPolicy,
  ServiceAccount, RBAC…). (PP-02)
- **YAML verbosity** and stringly-typed fields (quantities, ports by name). (PP-03)
- **Resource overhead** on small clusters and edge. (PP-04)
- **Upgrade treadmill**: 3 releases/year, one-minor-at-a-time upgrades, API removals.
  (PP-05)
- **Security defaults** historically permissive (no seccomp default, no userns, auto-mounted
  SA tokens, unencrypted secrets at rest by default). (PP-06)

## kube-apiserver

| Pros | Cons |
|---|---|
| Generic machinery for all kinds; strong consistency via etcd | Memory grows with cluster size (watch cache holds every object, plus etcd) (PP-10) |
| Watch cache dramatically reduces etcd load | Large LISTs historically caused OOMs; mitigated by consistent reads from cache and WatchList, still evolving (PP-11) |
| Priority & Fairness protects against noisy clients | APF config is complex; defaults often fine but opaque (PP-12) |
| Server-side apply with field management | managedFields bloat objects (often larger than spec) (PP-13) |
| Admission webhooks are flexible | Webhooks couple API availability to arbitrary pods; latency; security risks (PP-14) |
| Aggregation layer for extension APIs | Broken aggregated APIs break discovery and namespace deletion (PP-15) |
| JSON/YAML human-friendly | JSON for CRDs is slow to encode/decode at scale (PP-16) |

## etcd
See [storage and etcd](04-storage-and-etcd-pros-cons.md). Summary: rock-solid Raft, but
size ceilings, defrag chores, separate operational burden, and duplicated caching with the
apiserver. (PP-20..23)

## kube-scheduler

| Pros | Cons |
|---|---|
| Well-designed plugin framework with clear extension points | Throughput limited (one pod per cycle; sampling to cope) (PP-30) |
| Rich constraints (affinity, spread, taints) | Inter-pod affinity historically slow (PP-31) |
| Preemption and priority | No gang scheduling / queues in core (Kueue, Volcano needed) (PP-32) |
| Profiles for different policies | No NUMA/topology awareness → kubelet admission failures (PP-33) |
| | Decisions never revisited (needs descheduler) (PP-34) |
| | Poor default explanations for unschedulable pods at scale (PP-35) |

## kube-controller-manager

| Pros | Cons |
|---|---|
| Many robust controllers, battle-tested | Single leader for all controllers – vertical scaling only (PP-40) |
| Expectations pattern handles stale caches | No fencing on writes after losing leadership (PP-41) |
| GC with propagation policies | GC builds a graph of the whole cluster; discovery-dependent (PP-42) |
| | Stuck finalizers/namespaces are a top operational complaint (PP-43) |

## kubelet

| Pros | Cons |
|---|---|
| Resilient node agent; static pods; checkpointing for some managers | PLEG relisting caused "PLEG is not healthy" at density (event-based PLEG later) (PP-50) |
| CRI abstraction allowed runtime choice | CRI is lowest-common-denominator; features need cross-project coordination (PP-51) |
| Rich probe types | Exec probes are expensive; timeouts were not enforced until 1.20 (PP-52) |
| Eviction manager protects nodes | Polling-based (10 s), page-cache-confused memory signals, kernel OOM often wins (PP-53) |
| CPU/memory/topology managers | Three managers + topology manager hints = complexity; policy changes require state file deletion and restart (PP-54) |
| Graceful node shutdown | Default max-pods 110 (structural, not hardware) (PP-55) |
| | Sidecar ordering absent until 1.28 (PP-56) |
| | `subPath` config mounts don't update (PP-57) |
| | Container log format text-based, no rate limit (PP-58) |
| | Dynamic config removed; config changes need restarts (PP-59) |

## kube-proxy
See [networking pros/cons](03-networking-pros-cons.md). Summary: iptables mode O(n),
conntrack issues, DNS races; IPVS/nftables improve; eBPF replacements are the real fix.
(PP-60..64)

## Pods and workloads

| Pros | Cons |
|---|---|
| Pod abstraction (co-scheduled containers, shared netns) is a great unit | Pause container overhead/indirection (PP-70) |
| Deployment/StatefulSet/DaemonSet/Job cover most needs | ReplicaSet layer leaks into UX (PP-71) |
| Rolling updates with surge/unavailable | No native canary/blue-green; no auto-rollback on progress deadline (PP-72) |
| PDBs and Eviction API | Drains are client-side logic in kubectl; unhealthy pods block drains by default (PP-73) |
| Jobs grew powerful (indexed, failure policies) | Template hash changes on control-plane upgrades can trigger mass rollouts (PP-74) |
| StatefulSets give stable identity | StatefulSet pods on dead nodes need manual force-delete (PP-75) |
| HPA works well for CPU | VPA requires pod restarts (until in-place resize) (PP-76) |
| | Graceful shutdown requires `preStop: sleep` folklore to avoid dropped requests (PP-77) |

## Security

| Pros | Cons |
|---|---|
| RBAC is solid and expressive | Hidden escalation paths (create pods → any SA/secret in ns) (PP-80) |
| Pod Security Admission (simple levels) | seccomp Unconfined by default; userns opt-in; privileged too easy (PP-81) |
| Bound SA tokens, TLS bootstrap | Certificate expiry outages (1-year default) (PP-82) |
| Encryption at rest with KMS v2 | Not enabled by default; `list secrets` returns values (PP-83) |
| | No tenancy object; multi-tenancy is DIY (PP-84) |
| | No built-in image signature verification (PP-85) |

## Networking/Storage
See dedicated docs [03](03-networking-pros-cons.md) and [04](04-storage-and-etcd-pros-cons.md).

## Developer and operator experience

| Pros | Cons |
|---|---|
| kubectl is powerful and scriptable | "Why is my pod not running?" requires correlating several commands (PP-90) |
| Strong conventions (labels, conditions) | Unknown fields silently dropped for years (PP-91) |
| Helm/Kustomize ecosystems | Helm templating error-prone; release state in Secrets; pruning was alpha for years (PP-92) |
| | kubeconfig merging/contexts confusion (PP-93) |
| | Events are lossy and stored in etcd (PP-94) |

## What Kubernetes does better than we will (for a long time)
- Ecosystem breadth: operators, charts, integrations, managed offerings.
- Conformance: a certification suite ensuring portability across vendors.
- Battle-testing at enormous scale across thousands of organisations.
- Documentation and community knowledge.

Rustwain's honest position: a learning project and a design exploration. Interop
adapters (OCI, CSI, KMS, manifest import) matter more than novel features for anyone
trying it.
