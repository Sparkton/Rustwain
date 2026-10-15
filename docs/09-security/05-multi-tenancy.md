# Multi-Tenancy

**K8s analogue:** namespaces as soft tenancy boundary, Hierarchical Namespace Controller
(HNC, archived), Capsule, vCluster (virtual control planes), Kamaji, multi-tenancy
working group guidance, cluster-per-tenant.

## Kubernetes' position
Kubernetes has no tenant concept. Teams compose: namespaces + RBAC + ResourceQuota +
LimitRange + NetworkPolicy + Pod Security Admission + node pools/taints + maybe
gVisor/Kata. Getting it right requires many objects per tenant and is easy to get wrong.
Hard multi-tenancy (untrusted tenants) is generally discouraged on shared clusters –
people use cluster-per-tenant (expensive) or virtual clusters.

## Rustwain: Tenant as a first-class object

```yaml
kind: Tenant
metadata: { name: acme }
spec:
  admins: { groups: ["acme-admins"] }
  namespaces: { allowed: ["acme-*"], max: 20 }
  isolation:
    minimumClass: microvm          # all pods at least microvm
    dedicatedNodes: false          # or a NodePool exclusively for this tenant
    coreScheduling: true           # no SMT sharing with other tenants
  network:
    baseline: DenyCrossTenant      # baseline policy tier
    egress: { allow: ["0.0.0.0/0"], deny: ["169.254.169.254/32"] }
  quota: { requests.cpu: "200", requests.memory: 800Gi, count/pods: 2000, store.bytes: 2Gi }
  securityLevel: restricted
  priorityClasses: [normal, batch-low]
  apiFairness: { level: default, shareWeight: 1 }
status: { namespaces: [...], usage: {...} }
```

The tenant controller materialises: namespace ownership, RBAC bindings for tenant admins,
baseline network policies, quota objects, default security level, admission constraints.
Tenant admins self-serve namespaces within limits.

## Isolation layers per tenancy level

| Layer | Soft (trusted teams) | Hard (untrusted / customers) |
|---|---|---|
| API objects | Namespace RBAC | + tenant-scoped API fairness, object quotas, metadata-only secret listing, no cluster-scoped read |
| Compute | process class + userns | microvm class, core scheduling, optional dedicated nodes |
| Network | baseline allow within tenant | default deny cross-tenant, egress controls, metadata endpoint blocked |
| Storage | storage classes per tenant | separate storage pools/encryption keys |
| DNS | cluster DNS | DNS views: tenant can't resolve other tenants' service names `[proposed]` |
| Images | shared node cache | per-tenant pull verification (ensure-secret-pulled-images semantics) |
| Logs/exec | namespace RBAC | same |
| Control-plane DoS | — | write rate limits, fairness weights |
| Side channels | — | core scheduling, no KSM, dedicated nodes for highest assurance |

## Information leakage via API
Kubernetes lets any authenticated user list namespaces? (Only with permission – but many
clusters grant broad `list namespaces`.) Cluster-scoped objects (nodes, storage classes,
priority classes) are often visible to everyone. In Rustwain, tenant users see:
- Their tenant's namespaces only (filtered list).
- Nodes only via a redacted view (`NodeSummary`: capacity class, zone – no IPs/hostnames)
  if at all.

## Virtual control planes?
vCluster-style (a control plane per tenant running inside the host cluster) gives tenants
full cluster-admin of their virtual cluster, CRDs of their own, etc. Interesting `[later]`:
with a lightweight embedded server, a "tenant control plane" could be a `rustwain server`
running as a pod with a syncer. Not in the initial scope; the Tenant object covers most
needs.

## Open questions

- DNS views per tenant – complexity vs benefit.
- Tenant-scoped custom types (tenant A installs a CRD without affecting tenant B) – hard
  (Kubernetes CRDs are cluster-scoped, a big multi-tenancy gap). Possibly allow
  tenant-scoped CustomTypes with group names prefixed by tenant.

## Build checklist

- [ ] Tenant kind + controller materialising child objects.
- [ ] Tenant-aware admission (minimum isolation, security level).
- [ ] Filtered list/visibility for tenant users.
- [ ] Core scheduling cookie per tenant in agent.
