# Services and Load Balancing

**K8s analogue:** Service (ClusterIP, NodePort, LoadBalancer, ExternalName, headless),
EndpointSlice, kube-proxy, session affinity, topology-aware routing,
`internalTrafficPolicy`/`externalTrafficPolicy`, `trafficDistribution`.

## Service object

```yaml
kind: Service
metadata: { name: api, namespace: shop }
spec:
  selector: { app: api }
  ports: [ { name: https, port: 443, targetPort: 8443, protocol: TCP } ]
  type: ClusterIP            # ClusterIP | NodePort | LoadBalancer | ExternalName | Headless
  ipFamilies: [IPv6, IPv4]
  affinity: None             # None | ClientIP{timeout}
  distribution: PreferSameZone   # Any | PreferSameNode | PreferSameZone
  internalPolicy: Cluster    # Cluster | Local
  externalPolicy: Cluster    # Cluster | Local (preserve client IP)
  algorithm: Random          # Random | Maglev | LeastRequest(L7 only)
status:
  clusterIPs: [fd00:10:96::1a, 10.96.0.26]
```

## Endpoint sets

Kubernetes moved from `Endpoints` (one object per service with all addresses – huge
objects, every change rewrites and re-sends everything; a 5,000-endpoint service update
fans out megabytes to every node) to `EndpointSlice` (chunks of ≤100 endpoints).

Rustwain `EndpointSet`:
- Sliced (≤ 128 endpoints per slice), slices keyed stably so that a single pod change
  modifies one slice.
- Each endpoint: `{ ip(s), node, zone, ready, serving, terminating, port mapping, weight,
  pod ref }`.
- **Delta watch** `[proposed]`: agents receive per-endpoint diffs rather than whole
  slices – the watch hub can compute diffs since it holds old and new revisions.
- Managed by the endpoint controller; also writable directly for selector-less services
  (external backends).

## ClusterIP allocation
- Service CIDR (e.g. `fd00:10:96::/112`, `10.96.0.0/16`), allocated by the API server
  at create time (strongly consistent – no duplicates), via an allocation bitmap stored in
  the store (Kubernetes 1.31+ uses `IPAddress` objects per allocated IP; either works).
- Option: **no ClusterIPs at all** `[open]` – with socket-level LB and DNS, a service
  could be addressed by name only, resolving to a per-service VIP from a large v6 range
  computed by hashing (no allocation). Interesting simplification; keep allocations for
  compatibility initially.

## Data path summary
See [eBPF dataplane](03-ebpf-dataplane.md):
- ClusterIP from pods: socket LB at connect/sendmsg.
- From microVM pods: per-packet DNAT at TAP host side.
- NodePort / LoadBalancer from outside: XDP/TC on physical NIC.

## Backend selection algorithms

| Algorithm | Behaviour | Use |
|---|---|---|
| Random | Uniform among ready endpoints | Default |
| Maglev | Consistent hashing (Google Maglev), minimal disruption when backends change | External LB, connection-sticky without state |
| ClientIP affinity | LRU map client→backend with timeout | Legacy session stickiness |
| Weighted | Per-endpoint weights | Canaries, heterogeneous backends |
| LeastRequest / P2C | Needs L7 or connection counts | Gateway layer only |

## Topology-aware routing

- `PreferSameNode`: route to a local endpoint if one is ready (DaemonSet-style local
  services, e.g. node-local caches). Kubernetes added `trafficDistribution: PreferSameNode`
  in 1.33.
- `PreferSameZone`: endpoints in the client's zone, fall back to all when none (or when
  local zone capacity is too small – Kubernetes' older "topology aware hints" tried to
  proportionally allocate and was confusing; the simple "prefer same zone, fall back if
  none ready" is what people want, with an overload threshold `[proposed]`).
- Implemented by giving each node a filtered/prioritised backend list per service in
  maps – computed on the node from endpoint zone info.

## externalPolicy: Local and client IP preservation
- `Cluster`: traffic arriving at any node may be forwarded to any node → SNAT needed for
  return path symmetry (unless DSR) → client IP lost.
- `Local`: only nodes with local endpoints accept; no SNAT; client IP preserved; LB
  health checks must target only nodes with endpoints (health check port served by agent
  reporting local endpoint count – same as Kubernetes `healthCheckNodePort`).
- **DSR (Direct Server Return)** `[proposed]`: forward to remote backend with original
  client IP (carrying the VIP in an IP option / Geneve option), the backend's node replies
  directly to the client. Preserves client IP with `Cluster` policy. Cilium supports DSR;
  requires underlay that accepts asymmetric routing.

## LoadBalancer services
- `LoadBalancer` type requires something to provision an external IP:
  - Cloud plugin (see [cloud provider](../10-cluster-operations/09-cloud-provider-integration.md)).
  - **Built-in bare-metal LB** `[proposed]`: like MetalLB/kube-vip –
    L2 mode (one node answers ARP/NDP for the VIP, failover via leader election) or BGP mode
    (all nodes with endpoints announce the VIP /32 to routers → ECMP). Address pools via
    `LoadBalancerPool` objects.
- External IP assignment written to `status.loadBalancer.ingress`.

## Headless and ExternalName
- **Headless** (`clusterIP: None`): no VIP; DNS returns pod IPs directly (StatefulSets).
- **ExternalName**: DNS CNAME to an external name. Kept for compatibility, but warn: it
  bypasses policy and can be abused to point internal names elsewhere (Kubernetes
  CVE-2020-8554-adjacent concerns). Gated by policy.
- `externalIPs` field (arbitrary IPs claimed by a service): **not supported** – it was the
  core of CVE-2020-8554 (MITM by claiming external IPs); Kubernetes recommends blocking
  it by admission. We just don't have it; LoadBalancer pools replace it.

## Graceful endpoint draining
- Endpoints carry `terminating` + `serving` flags (Kubernetes 1.26+ semantics):
  terminating-but-still-serving endpoints are used only when no non-terminating
  endpoints exist (avoids black-holing during full rollouts with `Local` policy).
- Existing connections to a removed backend: socket-LB connections are already
  established to the pod IP and continue until closed (good). Per-packet DNAT paths keep
  `ct` entries until timeout.
- See [pod lifecycle](../04-node-agent/02-pod-lifecycle.md) for the drain gate.

## Scale targets
- 10,000 services, 100,000 endpoints, 5,000 nodes.
- Endpoint change → all nodes programmed: p99 < 1 s.
- Per-node CPU for applying 1,000 endpoint changes/s: < 5% of one core.

## Open questions

- VIP-less services.
- Delta watch for endpoint sets – API complexity vs bandwidth saving (significant at scale:
  Kubernetes at 5k nodes sends each slice update to 5k watchers).

## Build checklist

- [ ] Service + EndpointSet types, ClusterIP allocator.
- [ ] Endpoint controller (selector → slices, stable slicing).
- [ ] Agent service syncer → BPF maps; topology-filtered backend lists.
- [ ] NodePort/LoadBalancer XDP/TC path; `Local` health check server.
- [ ] Bare-metal L2/BGP LB `[later]`.
- [ ] Scale test with simulated nodes (watch fan-out + map programming).
