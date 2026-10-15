# Kubernetes Feature Coverage Matrix

A checklist of everything Kubernetes (and its de-facto ecosystem) does, mapped to the
Rustwain document that covers it and the planned approach. The point is to make sure
nothing is silently missing. If you think of a Kubernetes feature not on this list, add
a row.

Legend for **Plan**: `core` = built into Rustwain; `plugin` = extension point, not in core;
`adapter` = compatibility layer; `later` = acknowledged, scheduled after the core
milestones; `skip` = deliberately not doing, with reason.

## API machinery

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| REST API, resource groups/versions | core | [api-server](../07-control-plane/01-api-server.md) | gRPC canonical + JSON gateway |
| OpenAPI schema, discovery | core | [api-server](../07-control-plane/01-api-server.md) | Generated from Rust types |
| Namespaces | core | [object-model](../07-control-plane/03-object-model.md) | Plus a first-class Tenant above them |
| Labels, selectors, annotations | core | [object-model](../07-control-plane/03-object-model.md) | Indexed label queries server-side |
| Field selectors | core | [watch-and-caching](../07-control-plane/04-watch-and-caching.md) | Arbitrary indexed fields, not a fixed allowlist |
| Server-side apply, managed fields | core | [object-model](../07-control-plane/03-object-model.md) | Compact field ownership encoding |
| Strategic merge patch, JSON patch | adapter | [object-model](../07-control-plane/03-object-model.md) | JSON merge patch + apply only in core |
| resourceVersion, optimistic concurrency | core | [state-store](../07-control-plane/02-state-store.md) | Revision = Raft index |
| Watch, bookmarks, resumption | core | [watch-and-caching](../07-control-plane/04-watch-and-caching.md) | |
| List pagination (continue tokens) | core | [watch-and-caching](../07-control-plane/04-watch-and-caching.md) | |
| Consistent reads from cache | core | [watch-and-caching](../07-control-plane/04-watch-and-caching.md) | Read-index based |
| Subresources (status, scale, exec, log) | core | [api-server](../07-control-plane/01-api-server.md) | |
| CustomResourceDefinitions | core | [extensibility](../07-control-plane/10-extensibility.md) | Schema-validated custom types |
| CRD conversion webhooks | core | [extensibility](../07-control-plane/10-extensibility.md) | In-process conversion where possible |
| API aggregation layer | plugin | [extensibility](../07-control-plane/10-extensibility.md) | Proxied API groups |
| Admission webhooks (mutating/validating) | core + plugin | [admission](../07-control-plane/07-admission-and-policy.md) | In-process policy first; webhooks with strict timeouts |
| ValidatingAdmissionPolicy (CEL) | core | [admission](../07-control-plane/07-admission-and-policy.md) | Expression language evaluated in-process |
| API Priority and Fairness | core | [api-fairness](../07-control-plane/09-api-fairness.md) | |
| Audit logging | core | [audit-and-events](../10-cluster-operations/10-audit-and-events.md) | |
| Events API | core | [audit-and-events](../10-cluster-operations/10-audit-and-events.md) | Stored outside the main keyspace |
| Owner references, garbage collection | core | [gc-and-finalizers](../07-control-plane/08-garbage-collection-and-finalizers.md) | |
| Finalizers | core | [gc-and-finalizers](../07-control-plane/08-garbage-collection-and-finalizers.md) | With stuck-finalizer visibility |
| Leases, leader election | core | [controllers](../07-control-plane/05-controllers.md) | Store-native leases |
| Encryption at rest | core | [secrets](../09-security/03-secrets.md) | |
| API deprecation policy | core | [upgrades](../10-cluster-operations/02-upgrades-and-compat.md) | |
| Kubernetes YAML import | adapter | [cli-and-ux](../10-cluster-operations/05-cli-and-ux.md) | `rwctl import` translates common kinds |

## Workloads

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| Pod (multi-container) | core | [workload-api](../08-workloads/01-workload-api.md) | |
| Init containers | core | [pod-lifecycle](../04-node-agent/02-pod-lifecycle.md) | |
| Sidecar containers (native, restartable init) | core | [sidecars-and-debugging](../04-node-agent/08-sidecars-and-debugging.md) | Ordering is first-class from day one |
| Ephemeral / debug containers | core | [sidecars-and-debugging](../04-node-agent/08-sidecars-and-debugging.md) | |
| Static pods | core | [node-agent-architecture](../04-node-agent/01-node-agent-architecture.md) | Directory-sourced pods |
| ReplicaSet | core | [workload-api](../08-workloads/01-workload-api.md) | Possibly folded into Deployment revisions |
| Deployment | core | [workload-api](../08-workloads/01-workload-api.md) | |
| StatefulSet | core | [workload-api](../08-workloads/01-workload-api.md) | Ordinal identity, per-replica claims |
| DaemonSet | core | [workload-api](../08-workloads/01-workload-api.md) | |
| Job, CronJob | core | [workload-api](../08-workloads/01-workload-api.md) | Indexed jobs, success policies |
| Pod disruption budgets | core | [rollouts-and-disruption](../08-workloads/02-rollouts-and-disruption.md) | |
| Rolling update, maxSurge/maxUnavailable | core | [rollouts-and-disruption](../08-workloads/02-rollouts-and-disruption.md) | |
| Canary / blue-green (Argo Rollouts) | core | [rollouts-and-disruption](../08-workloads/02-rollouts-and-disruption.md) | Rollout object in core |
| HPA | core | [autoscaling](../08-workloads/03-autoscaling.md) | |
| VPA | later | [autoscaling](../08-workloads/03-autoscaling.md) | In-place resize makes it easier |
| In-place pod resize | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | cgroup writes, no restart |
| Cluster autoscaler / Karpenter | plugin | [autoscaling](../08-workloads/03-autoscaling.md) | Node provider interface |
| Gang scheduling, queues (Kueue, Volcano) | core | [batch-and-gang](../08-workloads/04-batch-and-gang-scheduling.md) | |
| ConfigMaps | core | [config-delivery](../08-workloads/05-config-and-secret-delivery.md) | |
| Downward API | core | [config-delivery](../08-workloads/05-config-and-secret-delivery.md) | |
| Projected volumes | core | [config-delivery](../08-workloads/05-config-and-secret-delivery.md) | |
| Lifecycle hooks (postStart/preStop) | core | [pod-lifecycle](../04-node-agent/02-pod-lifecycle.md) | |
| terminationGracePeriod, termination messages | core | [pod-lifecycle](../04-node-agent/02-pod-lifecycle.md) | |
| Pod readiness gates | core | [health-probes](../04-node-agent/03-health-probes.md) | |
| Pod overhead | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | Automatic per isolation class |
| Windows containers | skip | – | Linux only |

## Scheduling & placement

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| Resource requests/limits | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | |
| Node selectors, node affinity | core | [placement](../08-workloads/06-placement.md) | |
| Pod affinity / anti-affinity | core | [placement](../08-workloads/06-placement.md) | Optimised implementation |
| Taints and tolerations | core | [placement](../08-workloads/06-placement.md) | |
| Topology spread constraints | core | [placement](../08-workloads/06-placement.md) | |
| Priority classes, preemption | core | [quotas-priority](../08-workloads/07-quotas-limits-priority.md) | |
| Scheduler profiles / plugins | core | [scheduler](../07-control-plane/06-scheduler.md) | Rust traits, compiled in |
| Scheduler extenders | plugin | [scheduler](../07-control-plane/06-scheduler.md) | gRPC extender, discouraged |
| Descheduler | later | [scheduler](../07-control-plane/06-scheduler.md) | Rebalancing controller |
| Dynamic Resource Allocation (DRA) | core | [devices](../03-virtualization/05-devices-and-passthrough.md) | Structured device claims |
| Extended resources, device plugins | core | [devices](../03-virtualization/05-devices-and-passthrough.md) | |
| Topology manager, CPU manager, memory manager | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | NUMA-aware |
| Hugepages | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | |

## Node

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| kubelet | core | [node-agent](../04-node-agent/01-node-agent-architecture.md) | Merged with runtime/proxy/CNI |
| CRI | skip (internal trait) | [runtime-architecture](../02-container-runtime/01-runtime-architecture.md) | In-process; CRI adapter `[open]` |
| RuntimeClass | core | [isolation-spectrum](../03-virtualization/01-isolation-spectrum.md) | Isolation class |
| Probes | core | [health-probes](../04-node-agent/03-health-probes.md) | |
| Node leases, heartbeats | core | [registration](../04-node-agent/05-node-registration-and-heartbeat.md) | |
| Node problem detector | core | [registration](../04-node-agent/05-node-registration-and-heartbeat.md) | Built-in condition checks |
| Eviction (memory/disk/pid pressure) | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | PSI-driven |
| Graceful node shutdown | core | [registration](../04-node-agent/05-node-registration-and-heartbeat.md) | systemd inhibitor locks |
| Image GC, container GC | core | [image-and-disk-gc](../04-node-agent/07-image-and-disk-gc.md) | |
| Container logs, rotation | core | [logs-exec](../04-node-agent/06-logs-exec-attach-port-forward.md) | |
| exec, attach, port-forward, cp | core | [logs-exec](../04-node-agent/06-logs-exec-attach-port-forward.md) | |
| Swap support | core | [resources-qos-eviction](../04-node-agent/04-resources-qos-eviction.md) | Per-QoS swap policy |
| User namespaces for pods | core (default on) | [rootless-and-userns](../02-container-runtime/04-rootless-and-userns.md) | |
| Credential providers (image pull) | plugin | [images](../02-container-runtime/03-images-and-snapshotters.md) | |
| Node feature discovery | core | [host-variance](../01-linux-foundations/05-linux-host-variance.md) | |

## Networking

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| Pod networking (CNI) | core | [pod-networking](../05-networking/02-pod-networking.md) | Built-in; CNI plugin host `[open]` |
| Services: ClusterIP, NodePort, LoadBalancer, ExternalName, headless | core | [services](../05-networking/04-services-and-load-balancing.md) | |
| kube-proxy | core | [services](../05-networking/04-services-and-load-balancing.md) | eBPF socket-LB + TC |
| EndpointSlices | core | [services](../05-networking/04-services-and-load-balancing.md) | |
| Topology-aware routing, traffic distribution | core | [services](../05-networking/04-services-and-load-balancing.md) | |
| Session affinity | core | [services](../05-networking/04-services-and-load-balancing.md) | |
| CoreDNS | core | [dns](../05-networking/05-dns-and-discovery.md) | Node-local, in-agent |
| NetworkPolicy | core | [network-policy](../05-networking/06-network-policy.md) | Identity-based, L7 later |
| Ingress | adapter | [ingress-gateway](../05-networking/07-ingress-and-gateway.md) | |
| Gateway API | core | [ingress-gateway](../05-networking/07-ingress-and-gateway.md) | |
| Dual-stack, IPv6 | core | [ipam](../05-networking/08-ipam-and-dual-stack.md) | IPv6-first |
| Service mesh (Istio/Linkerd) | plugin + core mTLS | [service-mesh](../05-networking/10-service-mesh.md) | Node-level mTLS, no sidecars |
| Multi-cluster services | later | [encryption-multicluster](../05-networking/09-encryption-and-multicluster.md) | |
| Egress gateways | later | [network-policy](../05-networking/06-network-policy.md) | |
| Bandwidth limits | core | [ebpf-dataplane](../05-networking/03-ebpf-dataplane.md) | EDT-based |

## Storage

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| emptyDir, hostPath | core | [storage-model](../06-storage/01-storage-model.md) | hostPath gated by policy |
| PV, PVC, StorageClass | core | [storage-model](../06-storage/01-storage-model.md) | Claim/Disk/StorageClass |
| Dynamic provisioning | core | [volume-driver](../06-storage/02-volume-driver-interface.md) | |
| CSI | adapter | [volume-driver](../06-storage/02-volume-driver-interface.md) | Host real CSI drivers |
| Volume snapshots, clones | core | [local-storage](../06-storage/03-local-storage.md) | |
| Volume expansion | core | [storage-model](../06-storage/01-storage-model.md) | |
| Generic ephemeral volumes | core | [storage-model](../06-storage/01-storage-model.md) | |
| Local persistent volumes | core | [local-storage](../06-storage/03-local-storage.md) | |
| Storage capacity tracking | core | [storage-model](../06-storage/01-storage-model.md) | |
| Volume health monitoring | later | [storage-model](../06-storage/01-storage-model.md) | |
| ReadWriteOncePod | core | [storage-model](../06-storage/01-storage-model.md) | Default for exclusive |
| Image volumes (OCI artifacts as volumes) | core | [images](../02-container-runtime/03-images-and-snapshotters.md) | |

## Security

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| TLS bootstrap, CSR API | core | [identity-pki](../09-security/01-identity-and-pki.md) | |
| Certificate rotation | core | [identity-pki](../09-security/01-identity-and-pki.md) | Automatic, short-lived |
| Service accounts, bound tokens | core | [authn-authz](../09-security/02-authn-authz.md) | |
| OIDC auth | core | [authn-authz](../09-security/02-authn-authz.md) | |
| RBAC | core | [authn-authz](../09-security/02-authn-authz.md) | |
| Node authorizer | core | [authn-authz](../09-security/02-authn-authz.md) | Agents see only their own objects |
| Pod Security Standards / admission | core | [workload-security-policy](../09-security/06-workload-security-policy.md) | |
| seccomp, AppArmor, SELinux | core | [security-primitives](../01-linux-foundations/03-security-primitives.md) | Default seccomp on |
| Secrets, KMS | core | [secrets](../09-security/03-secrets.md) | |
| Image signature verification (sigstore/policy controllers) | core | [supply-chain](../09-security/04-supply-chain.md) | |
| Workload identity (SPIFFE) | core | [identity-pki](../09-security/01-identity-and-pki.md) | |

## Operations & ecosystem

| Kubernetes feature | Plan | Where | Notes |
|---|---|---|---|
| kubeadm / k3s-style install | core | [bootstrap](../10-cluster-operations/01-bootstrap-and-join.md) | Single binary |
| Upgrades, version skew policy | core | [upgrades](../10-cluster-operations/02-upgrades-and-compat.md) | |
| etcd backup/restore | core | [backup-dr](../10-cluster-operations/07-backup-and-disaster-recovery.md) | Built-in snapshots |
| Velero-style app backup | later | [backup-dr](../10-cluster-operations/07-backup-and-disaster-recovery.md) | |
| Helm, Kustomize | adapter | [packaging-gitops](../10-cluster-operations/08-packaging-and-gitops.md) | Native overlays + Helm render import |
| GitOps (Argo CD, Flux) | plugin | [packaging-gitops](../10-cluster-operations/08-packaging-and-gitops.md) | Apply-set primitive in core |
| Operators (operator SDK, controller-runtime) | core | [extensibility](../07-control-plane/10-extensibility.md) | Rust controller SDK |
| Cloud controller manager | plugin | [cloud-provider](../10-cluster-operations/09-cloud-provider-integration.md) | |
| metrics-server, Metrics API | later | [observability](../10-cluster-operations/06-observability.md) | Deferred |
| Prometheus metrics endpoints | later | [observability](../10-cluster-operations/06-observability.md) | Deferred |
| Dashboard | skip | – | No frontend by scope |
| kubectl | core | [cli-and-ux](../10-cluster-operations/05-cli-and-ux.md) | `rwctl` |
| kubeconfig, contexts | core | [cli-and-ux](../10-cluster-operations/05-cli-and-ux.md) | |
| Federation / multi-cluster management | later | [encryption-multicluster](../05-networking/09-encryption-and-multicluster.md) | |
| Cluster API (declarative cluster lifecycle) | later | [bootstrap](../10-cluster-operations/01-bootstrap-and-join.md) | |
| Time sync expectations (NTP) | core | [host-variance](../01-linux-foundations/05-linux-host-variance.md) | Preflight check |

## Things Kubernetes leaves out that we want

| Gap | Where |
|---|---|
| First-class VMs alongside containers (KubeVirt-like) | [full-vm-workloads](../03-virtualization/04-full-vm-workloads.md) |
| Built-in microVM isolation | [kvm-and-microvms](../03-virtualization/02-kvm-and-microvms.md) |
| Native multi-tenancy object | [multi-tenancy](../09-security/05-multi-tenancy.md) |
| Built-in node-to-node encryption | [encryption](../05-networking/09-encryption-and-multicluster.md) |
| Built-in gang scheduling and queues | [batch](../08-workloads/04-batch-and-gang-scheduling.md) |
| Stuck-object diagnostics (`rwctl why`) | [cli-and-ux](../10-cluster-operations/05-cli-and-ux.md) |
| Performance budgets enforced in CI | [budgets](../11-efficiency/01-performance-budgets.md) |
