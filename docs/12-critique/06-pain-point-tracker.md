# Pain Point Tracker

The single place to see every known problem with Kubernetes and its ecosystem that
Rustwain intends to address (or deliberately doesn't), what we plan to do, and where we
are. Update the **Status** column as work lands. IDs are stable – reference them in
design docs, issues and commits (`PP-53`).

**Status values:** `designed` (doc exists) · `building` · `done` (implemented + tested) ·
`deferred` · `won't fix` (with reason) · `open` (no answer yet).

**Severity** (how much it hurts Kubernetes users in practice): `H` high · `M` medium · `L` low.

**Trade-off**: what we give up by fixing it our way.

## General

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-01 | Too many components/processes and config surfaces | H | 3 binaries (server, agent, rwctl) + tiny supervisor | Less pluggability at process level; bigger blast radius per process (mitigated by module supervision) | [architecture](../00-overview/03-system-architecture.md) | designed |
| PP-02 | Many object kinds before production readiness | M | Fewer, richer kinds (sidecars/canary/config-rollout built in); `rwctl lint` | Bigger individual kinds | [workload API](../08-workloads/01-workload-api.md) | designed |
| PP-03 | YAML verbosity, stringly-typed fields | M | Typed API, flatter placement API, strict decoding | Not drop-in compatible with k8s YAML (import adapter) | [object model](../07-control-plane/03-object-model.md) | designed |
| PP-04 | High overhead on small/edge clusters | H | Agent < 40 MB, server sized for small clusters | — | [budgets](../11-efficiency/01-performance-budgets.md) | designed |
| PP-05 | Upgrade treadmill, one-minor-at-a-time, disruptive node upgrades | H | N−2 agent skew, in-place agent upgrade with zero pod restarts, feature levels | Must maintain multi-version protocols | [upgrades](../10-cluster-operations/02-upgrades-and-compat.md) | designed |
| PP-06 | Permissive security defaults | H | userns + seccomp + NNP default; no auto token; encryption at rest default | Some images/apps need explicit opt-outs | [workload security](../09-security/06-workload-security-policy.md) | designed |

## API server

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-10 | apiserver + etcd double memory (watch cache + store) | H | Store's in-memory MVCC *is* the cache | Dataset must fit in RAM (bounded by quotas) | [state store](../07-control-plane/02-state-store.md) | designed |
| PP-11 | Large LISTs → memory spikes/OOM | H | Watch-list only for informers; lists always from cache; paging enforced | Clients must use streaming APIs | [watch](../07-control-plane/04-watch-and-caching.md) | designed |
| PP-12 | APF opaque | L | Simplified levels; `rwctl api fairness`; reason headers on 429 | Less tunability | [fairness](../07-control-plane/09-api-fairness.md) | designed |
| PP-13 | managedFields bloat | M | Compact bitset ownership, hidden by default | Custom encoding to maintain | [object model](../07-control-plane/03-object-model.md) | designed |
| PP-14 | Webhooks couple availability, add latency, CVE risk | H | In-process CEL policies; webhook guardrails + circuit breaker | CEL less expressive than arbitrary code | [admission](../07-control-plane/07-admission-and-policy.md) | designed |
| PP-15 | Broken aggregated APIs break discovery/namespace deletion | M | Discovery degrades per group; namespace deletion via index | — | [extensibility](../07-control-plane/10-extensibility.md) | designed |
| PP-16 | CRDs stored as JSON; slow at scale | M | Schema-driven binary encoding for custom types | Encoding complexity | [extensibility](../07-control-plane/10-extensibility.md) | designed |
| PP-17 | No referential integrity (pods referencing missing secrets just hang) | M | Typed refs + `MissingReference` blocking reason + warnings | — | [object model](../07-control-plane/03-object-model.md) | designed |
| PP-18 | Unknown fields silently dropped (historically) | M | Strict decoding errors by default | Less lenient for old manifests | [CLI](../10-cluster-operations/05-cli-and-ux.md) | designed |

## etcd / state

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-20 | Size quota cliff → cluster read-only | H | Explicit per-tenant/object quotas; alarms well before limits; in-memory store with snapshotting | Hard upper bound by RAM | [state store](../07-control-plane/02-state-store.md) | designed |
| PP-21 | Defrag/compaction chores | M | No defrag needed (in-memory + snapshot; log truncation) | Snapshot I/O periodically | [state store](../07-control-plane/02-state-store.md) | designed |
| PP-22 | Separate etcd operations (TLS, backups, upgrades) | H | Embedded store; built-in scheduled backups | Store bugs are our bugs (sim testing needed) | [backup](../10-cluster-operations/07-backup-and-disaster-recovery.md) | designed |
| PP-23 | Heartbeat/event write churn | M | Leader-local leases; separate lossy event store | Events not durable | [registration](../04-node-agent/05-node-registration-and-heartbeat.md), [events](../10-cluster-operations/10-audit-and-events.md) | designed |
| PP-24 | Restore resets revisions, confusing caches | L | Revision bump on restore | — | [backup](../10-cluster-operations/07-backup-and-disaster-recovery.md) | designed |
| PP-25 | Leader elections from fsync latency spikes (noisy/shared disks; "etcdserver: request timed out") | H | PreVote + check-quorum; group commit; leader fsync in parallel with replication; never retry fsync after EIO (step down) | Needs a dedicated log disk for best results | [state store](../07-control-plane/02-state-store.md) | designed |
| PP-26 | Stale or ambiguous reads (resourceVersion="0" served from a lagging watch cache; ambiguous write outcome on leader loss) | M | ReadIndex-gated reads from the in-memory store; explicit read levels; idempotency keys on proposals | Dedup table in the state machine | [state store](../07-control-plane/02-state-store.md) | designed |

## Scheduler

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-30 | Throughput limits | M | Parallel filtering, equivalence classes, batch placement | Cache invalidation risk | [scheduler](../07-control-plane/06-scheduler.md) | designed |
| PP-31 | Inter-pod affinity slow | M | Domain counters | Index memory | [placement](../08-workloads/06-placement.md) | designed |
| PP-32 | No gang/queues in core | H | Queues, gang, group-at-once placement built in | Scheduler complexity | [batch](../08-workloads/04-batch-and-gang-scheduling.md) | designed |
| PP-33 | NUMA/topology unknown to scheduler | M | Topology-aware scheduling, allocations in binding | Larger node objects | [resources](../04-node-agent/04-resources-qos-eviction.md) | designed |
| PP-34 | Decisions never revisited | M | Rebalancer controller | Extra evictions | [scheduler](../07-control-plane/06-scheduler.md) | deferred |
| PP-35 | Unhelpful unschedulable messages | M | Structured reasons + closest-node analysis | — | [scheduler](../07-control-plane/06-scheduler.md) | designed |
| PP-36 | Pods placed on wrong arch (exec format error) | M | Image-platform-derived constraints | Admission-time image index lookup | [heterogeneous](../10-cluster-operations/04-heterogeneous-fleets.md) | designed |

## Controllers

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-40 | Single leader for all controllers | L | Per-controller leases spread across servers | Harder to debug "where is X running" | [controllers](../07-control-plane/05-controllers.md) | designed |
| PP-41 | No write fencing for deposed leaders | M | Lease-token preconditions on writes | Slight write overhead | [controllers](../07-control-plane/05-controllers.md) | designed |
| PP-42 | GC builds whole-cluster graph | M | Owner index in store | Index maintained in state machine | [GC](../07-control-plane/08-garbage-collection-and-finalizers.md) | designed |
| PP-43 | Stuck finalizers / namespaces | H | Registered finalizers, progress status, `rwctl why`, audited force | Third-party controllers must register | [GC](../07-control-plane/08-garbage-collection-and-finalizers.md) | designed |

## Node agent

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-50 | PLEG relisting / "PLEG not healthy" | H | Push events from supervisors; no relisting | — | [node agent](../04-node-agent/01-node-agent-architecture.md) | designed |
| PP-51 | CRI lowest-common-denominator | M | In-process runtime trait | No drop-in runtime swapping (CRI adapter optional) | [runtime](../02-container-runtime/01-runtime-architecture.md) | designed |
| PP-52 | Exec probes expensive; timeouts | M | Timer wheel, two-level cgroups, socket-state/pressure probes | — | [probes](../04-node-agent/03-health-probes.md) | designed |
| PP-53 | Slow, inaccurate eviction (kernel OOM wins) | H | PSI triggers, graduated response, zero-grace emergency kill | Kernel ≥ PSI support required | [resources](../04-node-agent/04-resources-qos-eviction.md) | designed |
| PP-54 | CPU/memory/topology manager complexity | M | One resource manager; scheduler-visible exclusive CPUs | — | [resources](../04-node-agent/04-resources-qos-eviction.md) | designed |
| PP-55 | 110 pods/node default | M | 500+ supported; dynamic IP blocks | — | [density](../11-efficiency/02-density-and-packing.md) | designed |
| PP-56 | Sidecar ordering (pre-1.28) | M | Ordered init list with sidecars from day one | — | [sidecars](../04-node-agent/08-sidecars-and-debugging.md) | designed |
| PP-57 | subPath config mounts don't update | L | Symlink-path file mounts | — | [config](../08-workloads/05-config-and-secret-delivery.md) | designed |
| PP-58 | Text logs, no rate limits | M | Framed binary logs with index; per-pod rate limit | Log shippers need exporter | [logs](../04-node-agent/06-logs-exec-attach-port-forward.md) | designed |
| PP-59 | Config changes need restarts | L | Live-reloadable dynamic config subset | — | [node agent](../04-node-agent/01-node-agent-architecture.md) | designed |
| PP-5A | CFS throttling with CPU limits | H | Limits optional; burst; throttling visible in status | — | [cgroups](../01-linux-foundations/02-cgroups-v2.md) | designed |
| PP-5B | Page cache counted as working set → false evictions | M | anon/unreclaimable + PSI based signals; memory.high | — | [cgroups](../01-linux-foundations/02-cgroups-v2.md) | designed |
| PP-5C | `kubectl cp` needs tar in image | L | Native file API | — | [logs/exec](../04-node-agent/06-logs-exec-attach-port-forward.md) | designed |
| PP-5D | Disk accounting via du scans | M | Project quotas | Needs fs support | [filesystems](../01-linux-foundations/04-filesystems.md) | designed |
| PP-5E | OOMKilled misreported for child processes | L | memory.events oom_kill counter | — | [supervision](../01-linux-foundations/06-process-supervision.md) | designed |
| PP-5F | Node-pressure eviction ignores PodDisruptionBudgets and can over-evict (several pods for one pressure event) | M | One eviction in flight per resource at `Evict` level; prefer pods with budget headroom in ranking; zero-grace only at `Emergency` | Slower relief at moderate pressure | [eviction](../04-node-agent/04-resources-qos-eviction.md) | designed |
| PP-5G | kubelet crash/restart leaves orphaned state ("orphaned pod found, but volume paths are still present on disk", leaked cgroups/netns) | M | Intent-before-effect checkpoint, UID-named host artifacts, orphan GC before admitting new pods, crash-point test matrix | Every step must be idempotent | [pod lifecycle](../04-node-agent/02-pod-lifecycle.md) | designed |

## Networking

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-60 | kube-proxy iptables O(n) | H | eBPF maps, O(1) | Kernel requirements (nftables fallback) | [dataplane](../05-networking/03-ebpf-dataplane.md) | designed |
| PP-61 | conntrack exhaustion / races | H | Socket-LB, own ct only where needed | — | [dataplane](../05-networking/03-ebpf-dataplane.md) | designed |
| PP-62 | DNS ndots:5 amplification + 5 s timeouts | H | Node-local resolver with autopath, no NAT on DNS | Resolver in agent | [DNS](../05-networking/05-dns-and-discovery.md) | designed |
| PP-63 | MTU blackholes with overlays | M | Computed MTU + MSS clamping | — | [network model](../05-networking/01-network-model.md) | designed |
| PP-64 | NetworkPolicy gaps (deny, cluster, FQDN, logging) | H | 3 tiers, FQDN, audit mode, `rwctl net can` | — | [policy](../05-networking/06-network-policy.md) | designed |
| PP-65 | Graceful endpoint removal race | H | Drain gate: SIGTERM after dataplanes ack removal | Slightly slower terminations | [pod lifecycle](../04-node-agent/02-pod-lifecycle.md) | designed |
| PP-66 | Sidecar mesh overhead | H | Node-level L4 mTLS; waypoints for L7 | Shared node proxy | [mesh](../05-networking/10-service-mesh.md) | designed |
| PP-67 | Ingress annotation sprawl / CVEs | M | Gateway API model natively | — | [gateway](../05-networking/07-ingress-and-gateway.md) | designed |
| PP-68 | `externalIPs` MITM (CVE-2020-8554) | M | Field doesn't exist; LB pools instead | Compat | [services](../05-networking/04-services-and-load-balancing.md) | designed |
| PP-69 | NetworkManager/firewalld/networkd interference | M | Doctor checks + drop-ins | — | [host variance](../01-linux-foundations/05-linux-host-variance.md) | designed |
| PP-6A | Fixed per-node pod CIDR | M | Dynamic blocks; IPv6 /64 | More routes | [IPAM](../05-networking/08-ipam-and-dual-stack.md) | designed |
| PP-6B | Encryption requires add-ons | M | Built-in WireGuard | — | [encryption](../05-networking/09-encryption-and-multicluster.md) | designed |
| PP-6C | Client source IP lost on NodePort/LB (`externalTrafficPolicy: Cluster`), or uneven load with `Local` | M | DSR with Geneve option + Maglev, per-node capability probe and SNAT fallback | Needs cloud anti-spoofing disabled (PP-R2) | [dataplane](../05-networking/03-ebpf-dataplane.md) | designed |
| PP-6D | Pod IP leaks after runtime/kubelet crashes ("no IP addresses available in range") | M | UID-keyed allocations in the same txn as the intent; leak repair on agent start; quarantine | — | [IPAM](../05-networking/08-ipam-and-dual-stack.md) | designed |

## Workloads

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-70 | Pause container | L | Supervisor holds namespaces | — | [namespaces](../01-linux-foundations/01-namespaces.md) | designed |
| PP-71 | ReplicaSet leaks into UX | L | Revisions | Import adapter for RS | [workload API](../08-workloads/01-workload-api.md) | designed |
| PP-72 | No native canary; no auto rollback | M | Canary/blue-green in Deployment; onFailure policy | Bigger Deployment controller | [rollouts](../08-workloads/02-rollouts-and-disruption.md) | designed |
| PP-73 | Client-side drains; unhealthy pods block drains | M | Server-side Drain; AlwaysAllow unhealthy eviction default | — | [rollouts](../08-workloads/02-rollouts-and-disruption.md) | designed |
| PP-74 | Template hash changes on upgrade → mass rollouts | M | Hash user-specified template | Must store pre-default spec | [workload API](../08-workloads/01-workload-api.md) | designed |
| PP-75 | StatefulSet pods on dead nodes need manual force delete | M | Fencing-aware replacement | Requires fencing confidence | [workload API](../08-workloads/01-workload-api.md) | designed |
| PP-76 | VPA requires restarts | M | In-place resize from day one | — | [autoscaling](../08-workloads/03-autoscaling.md) | designed |
| PP-77 | preStop sleep folklore | H | Drain gate (see PP-65) | — | [pod lifecycle](../04-node-agent/02-pod-lifecycle.md) | designed |
| PP-78 | Config change doesn't roll pods | M | `onChange: Rollout` | — | [config](../08-workloads/05-config-and-secret-delivery.md) | designed |
| PP-79 | Mixed image versions when tags move mid-rollout | M | Digest pinning at admission | Registry access from control plane | [images](../02-container-runtime/03-images-and-snapshotters.md) | designed |
| PP-7A | Job with sidecar never completes (pre-1.28) | M | Sidecars stopped on main completion | — | [sidecars](../04-node-agent/08-sidecars-and-debugging.md) | designed |
| PP-7B | CronJob "too many missed start times" | L | Run once for missed windows | — | [workload API](../08-workloads/01-workload-api.md) | designed |

## Security

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-80 | Hidden RBAC escalation paths | H | `serviceaccounts/use`; `rwctl auth explain` with escalation warnings | Extra permission to grant | [authz](../09-security/02-authn-authz.md) | designed |
| PP-81 | Unconfined seccomp, no userns by default | H | Defaults on | Compat opt-outs | [security primitives](../01-linux-foundations/03-security-primitives.md) | designed |
| PP-82 | Certificate expiry outages | H | Short-lived auto-rotated certs; `rwctl pki status` | More frequent rotation traffic | [PKI](../09-security/01-identity-and-pki.md) | designed |
| PP-83 | Secrets not encrypted by default; list returns values | H | Encryption on; metadata-only list without `get-values` | Compat with tools listing secrets | [secrets](../09-security/03-secrets.md) | designed |
| PP-84 | No tenancy object | H | Tenant kind | New concept to learn | [multi-tenancy](../09-security/05-multi-tenancy.md) | designed |
| PP-85 | No built-in image verification | M | ImagePolicy with Sigstore | Registry access at admission | [supply chain](../09-security/04-supply-chain.md) | designed |
| PP-86 | Image pulled with creds reusable by other pods | M | Credential-aware image access checks | Extra registry HEAD | [images](../02-container-runtime/03-images-and-snapshotters.md) | designed |
| PP-87 | Long-lived SA tokens auto-mounted (legacy) | H | No auto-mount; bound short-lived only | — | [PKI](../09-security/01-identity-and-pki.md) | designed |
| PP-88 | Distro userns restrictions (Ubuntu 24.04+ AppArmor) break rootless/userns workloads with opaque EPERM | M | Empirical userns probes (root and unprivileged), doctor remediation, shipped AppArmor profile for rootless mode | One more distro-specific artifact to maintain | [kernel feature matrix](../01-linux-foundations/07-kernel-feature-matrix.md) | designed |

## Storage

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-A0 | CSI sidecar sprawl | M | Logic in our controllers; CSI on the wire | Must track CSI spec | [volume drivers](../06-storage/02-volume-driver-interface.md) | designed |
| PP-A1 | Multi-attach / double-mount after node loss | H | Fencing-gated detach | Slower failover for exclusive volumes | [storage model](../06-storage/01-storage-model.md) | designed |
| PP-A2 | Immediate binding zone mismatch | M | WaitForFirstConsumer default | — | [storage model](../06-storage/01-storage-model.md) | designed |
| PP-A3 | fsGroup recursive chown slowness | M | idmapped mounts | Kernel/fs support needed | [volume drivers](../06-storage/02-volume-driver-interface.md) | designed |
| PP-A4 | Local PVs awkward; node loss = stuck | M | Native local driver, capacity tracking, Abandon policy | — | [local storage](../06-storage/03-local-storage.md) | designed |
| PP-A5 | RWO allows multiple pods on node | M | ExclusivePod default | Compat | [storage model](../06-storage/01-storage-model.md) | designed |

## UX and operations

| ID | Pain point | Sev | Rustwain answer | Trade-off | Doc | Status |
|---|---|---|---|---|---|---|
| PP-90 | "Why isn't my pod running?" | H | `blocking` status + `rwctl why` | — | [CLI](../10-cluster-operations/05-cli-and-ux.md) | designed |
| PP-91 | Unknown fields dropped | M | Strict decoding (PP-18) | — | [CLI](../10-cluster-operations/05-cli-and-ux.md) | designed |
| PP-92 | Helm templating pitfalls; pruning | M | ApplySet + typed overlays; Helm via pipe | Less templating power | [packaging](../10-cluster-operations/08-packaging-and-gitops.md) | designed |
| PP-93 | kubeconfig confusion | L | Single config file, explicit contexts | — | [CLI](../10-cluster-operations/05-cli-and-ux.md) | designed |
| PP-94 | Events lossy & in etcd | M | Separate event store, explicitly lossy | Still lossy (by design) | [events](../10-cluster-operations/10-audit-and-events.md) | designed |
| PP-95 | Dead cloud nodes linger NotReady | L | NodeLifecycle plugin deletes | — | [cloud](../10-cluster-operations/09-cloud-provider-integration.md) | designed |
| PP-96 | Control-plane overload → false node deaths → reschedule storms | H | Leader-local leases; overload-aware node lifecycle; zone rate limits | — | [fairness](../07-control-plane/09-api-fairness.md) | designed |

## Rustwain-specific trade-offs (PP-R)

New pain that our own designs introduce. Kept here so the cost of each decision stays
visible next to the problem it solves.

| ID | Pain point | Sev | Mitigation | Caused by | Doc | Status |
|---|---|---|---|---|---|---|
| PP-R1 | PSI is compiled in but disabled by default on Debian and RHEL kernels; PSI-driven eviction needs a `psi=1` boot argument | M | `degraded` mode with 1 s usage polling; doctor prints the exact grub/grubby fix | PSI-first eviction (PP-53) | [kernel feature matrix](../01-linux-foundations/07-kernel-feature-matrix.md) | designed |
| PP-R2 | DSR replies leave a node with another node's source IP; cloud anti-spoofing and strict uRPF drop them | M | Per-node `dsr-capable` probe at join; automatic SNAT fallback per node | DSR (PP-6C) | [dataplane](../05-networking/03-ebpf-dataplane.md) | designed |
| PP-R3 | Leader-local leases stretch dead-node detection to ~2× lease duration after an unplanned leader failover | L | Lease-table handover on planned transfer; `[open]` replicated suspect list | Non-durable heartbeats (PP-23) | [state store](../07-control-plane/02-state-store.md) | designed |
| PP-R4 | MicroVM pods need per-packet NAT and conntrack (guest kernel bypasses socket LB) | M | Separate preallocated `ct` map for TAP endpoints; `CtFull` drops new flows, never established ones | MicroVM isolation class | [dataplane](../05-networking/03-ebpf-dataplane.md) | designed |
| PP-R5 | First use of a new affinity/spread selector costs an O(pods) backfill in the scheduler | L | Selector registry with refcounts; backfill off the scheduling hot path | Domain counters (PP-31) | [scheduler](../07-control-plane/06-scheduler.md) | designed |

## Things we deliberately won't fix (or not yet)

| ID | Item | Reason |
|---|---|---|
| PP-W1 | Kubernetes API wire compatibility | Constrains the model; import adapter instead; gateway maybe later |
| PP-W2 | Windows nodes | Out of scope (Linux only) |
| PP-W3 | Web dashboard | Out of scope (no frontend) |
| PP-W4 | Full observability stack | Deferred by project scope |
| PP-W5 | Arbitrary in-process plugins (WASM) | Out of scope; CEL + out-of-process extension |
| PP-W6 | Ecosystem breadth parity | Impossible short-term; prioritise interop adapters |

## How to use this tracker
1. When a design doc introduces a fix for a Kubernetes problem, add or reference a row.
2. When implementation starts, set `building`; when it's tested (including a regression
   test demonstrating the Kubernetes failure mode doesn't happen), set `done` and link the
   test.
3. If we discover our approach has its own new pain, add a row with prefix `PP-R`
   (Rustwain-specific) – honesty about our own trade-offs is the point.
