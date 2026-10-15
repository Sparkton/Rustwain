# Cloud Provider Integration

**K8s analogue:** cloud-controller-manager (node controller: addresses/zone labels and
node deletion on instance termination; route controller; service controller for
LoadBalancers), cloud CSI drivers, cloud IPAM (VPC CNI), cluster autoscaler cloud
providers, workload identity federation (IRSA, GKE WI, Azure WI).

## Principle
**No cloud SDKs in core binaries.** Clouds integrate through plugin interfaces; plugins are
separate binaries (gRPC over unix socket or as workloads).

Kubernetes learned this the hard way: in-tree cloud providers bloated the core and coupled
releases; the move out-of-tree took ~5 years.

## Plugin interfaces

| Interface | Responsibilities | Runs where |
|---|---|---|
| `NodeLifecycle` | Instance metadata (zone, region, instance type, addresses), detect terminated instances → delete Node, spot interruption notices | Server-side plugin (controller) |
| `LoadBalancer` | Ensure/update/delete external LB for `LoadBalancer` services; report ingress IPs/hostnames; health check configuration | Server-side plugin |
| `Routes` | Program VPC route tables with pod CIDR blocks (native routing without overlay) | Server-side plugin |
| `Ipam` | Allocate VPC IP blocks for pods (prefix delegation) | Server + agent |
| `NodeProvider` | Create/delete instances for autoscaling | Server-side plugin |
| Storage | Via CSI drivers (existing cloud CSI drivers work through our adapter) | Driver pods |
| Identity federation | Our OIDC issuer + cloud IAM trust configuration (documentation + helper commands) | n/a |
| Credential provider | Registry tokens (ECR/GAR/ACR) | Agent plugin |

## LoadBalancer controller semantics
- Watch Services of type `LoadBalancer` (with a `loadBalancerClass` selecting the plugin;
  multiple implementations can coexist – bare-metal built-in, cloud plugin).
- Ensure LB with listeners for service ports → targets:
  - **Instance mode**: targets = nodes:NodePort (works everywhere; extra hop).
  - **IP mode**: targets = pod IPs directly (requires VPC-routable pod IPs) – better.
- Health checks: `externalPolicy: Local` → agent health check port per service.
- Finalizer on Service to delete the cloud LB before the Service disappears (registered
  finalizer – see GC doc).
- Status reports provisioning progress and errors (`LoadBalancerPending{reason}`).

## Node lifecycle
- Agent can't know if its instance is being terminated; the plugin watches the cloud API
  (or instance metadata service for spot notices) and:
  - Taints/drains nodes ahead of termination.
  - Deletes Node objects for instances that no longer exist (otherwise dead nodes linger as
    `NotReady` forever – common in Kubernetes without CCM).
  - Provides **fencing** (stop instance) for the storage safety path.

## Reference plugins `[later]`
- `rustwain-cloud-aws`, `-gcp`, `-azure` as separate repos/crates; plus `rustwain-cloud-hetzner`/
  `-openstack` community-style. Not part of the initial roadmap; the interfaces are.
- Local testing provider (`libvirt`/Cloud Hypervisor VMs) implements NodeLifecycle +
  NodeProvider + LoadBalancer (via a userspace LB on the host) – lets us test cloud-like
  flows on a laptop.

## Build checklist
- [ ] gRPC plugin interface definitions (proto) for each interface.
- [ ] Plugin host in server (discovery, health, version negotiation).
- [ ] Local VM provider implementing NodeLifecycle/NodeProvider/LoadBalancer.
