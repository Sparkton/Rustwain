# Network Policy

**K8s analogue:** NetworkPolicy (L3/L4, namespace/pod selectors, ipBlocks),
AdminNetworkPolicy / BaselineAdminNetworkPolicy (cluster-scoped, SIG-network), Calico
GlobalNetworkPolicy, CiliumNetworkPolicy (L7, FQDN, identity-based).

## Kubernetes NetworkPolicy and its gaps

- Namespaced; selects pods; allow-list semantics: once any policy selects a pod for a
  direction, only explicitly allowed traffic is permitted in that direction.
- Peers: podSelector, namespaceSelector, ipBlock (CIDR with excepts).
- Gaps: no deny rules; no cluster-wide/admin policies (fixed by AdminNetworkPolicy,
  still not GA everywhere); no FQDN-based egress; no L7; no logging; semantics for
  host-network pods and node traffic undefined; implementations differ (some don't
  support `endPort`, some differ on named ports); enforcement on IP sets means policy
  scale = O(pods × rules) for IP-list implementations.

## Rustwain policy model

Three tiers evaluated in order (matching AdminNetworkPolicy's sane design):

1. **Cluster policy** (cluster admins): `Allow`, `Deny`, or `Pass` (delegate to lower tier),
   with explicit priorities. E.g. "deny all tenants → node metadata endpoint
   169.254.169.254", "allow monitoring namespace → all pods :9090".
2. **Tenant/namespace policy** (owners): allow-list semantics like Kubernetes
   NetworkPolicy, plus explicit `deny` rules.
3. **Baseline** (cluster admins): defaults when nothing else matched (e.g. default-deny
   cross-tenant, default-allow within tenant).

```yaml
kind: NetworkPolicy
metadata: { name: api-ingress, namespace: shop }
spec:
  selector: { app: api }
  ingress:
    - from: [ { pods: { app: web } }, { namespaces: { team: payments }, pods: { role: client } } ]
      ports: [ { port: 8443, protocol: TCP } ]
  egress:
    - to: [ { pods: { app: db } } ]
      ports: [ { port: 5432 } ]
    - to: [ { dns: "*.stripe.com" } ]             # FQDN egress
      ports: [ { port: 443 } ]
    - to: [ { cidr: 10.200.0.0/16, except: [10.200.5.0/24] } ]
```

## Identity-based enforcement

Instead of compiling selectors into IP lists on every node (which change on every pod
churn), use **security identities**:

- An identity = a hash of the *policy-relevant* label set of a pod (namespace + selected
  labels) → a 24/32-bit numeric ID allocated cluster-wide by the identity controller
  (stored in the API; or deterministically derived to avoid allocation round trips
  `[open]`).
- Pods with identical relevant labels share an identity (e.g. all 50 replicas of
  `app=api` in `shop` → one identity). Policy rules reference identities, so scaling a
  deployment changes **no policy state** – only `ipcache` (IP → identity).
- Policy compiler: for each local endpoint's identity, compute allowed (peer identity,
  port, proto) tuples → policy map entries. Changes when policies or the set of
  identities matching a selector change, not when pods churn.
- Identity carried in packet (Geneve option) or looked up from source IP in `ipcache`.

Special identities: `host` (this node), `remote-node`, `world` (outside cluster), `kube-api`
(servers), `health` (probes), `unmanaged`.

## Enforcement points
- **Egress** policy at the source pod's netkit/veth program (drop early, before using
  network).
- **Ingress** policy at the destination pod's host-side program.
- Both are checked (defence in depth – a compromised source node can't bypass ingress).
- **Per-flow**: full evaluation on connection setup; established flows use the `ct` map.
  Policy changes must re-evaluate existing flows `[open]`: Kubernetes semantics are
  undefined for existing connections; most implementations don't kill established ones.
  We: re-evaluate on next packet for flows whose cached verdict generation is old (cheap
  generation counter check).

## FQDN egress policy
"Allow egress to `*.stripe.com:443`" can't be compiled to IPs statically (they change).
Approach (same as Cilium): the node-local DNS resolver sees the pod's queries and their
answers → when a pod resolves an allowed FQDN, insert the returned IPs (with TTL) into the
pod's allowed-egress set. Requires DNS to go through our resolver (enforced by policy:
DNS to other resolvers blocked when FQDN rules exist).

## L7 policy `[later]`
HTTP method/path, gRPC service/method, Kafka topics. Requires terminating/inspecting the
connection: transparent proxy in the agent (redirect with `sk_lookup` or TPROXY to a
node-local proxy). Overlaps with the service mesh design (see
[service mesh](10-service-mesh.md)). Node-level proxy shared by all pods (Envoy per node in
Cilium; we'd write a Rust proxy – `pingora`/`hyper`-based, `[open]`).

## Host and node traffic
- Probes from the agent: always allowed (marked).
- Pod → node IPs and node → pod (non-probe): governed by policy with `host`/`remote-node`
  identities.
- Host firewall (protecting the node itself, e.g. only allow SSH from bastion) –
  `HostPolicy` `[later]`; BPF on physical NIC ingress.

## Policy logging and visibility
- Verdict counters per policy rule (map counters).
- Drop events with reason (`POLICY_DENIED{rule}`) to a ring buffer → `rwctl net trace
  pod/x` live view (Hubble-lite) `[later, with observability]`.
- **Policy dry-run / audit mode**: apply a policy in `audit` mode – evaluate and log what
  would be dropped without dropping. Critical for safely rolling out default-deny.
- `rwctl net can pod/a pod/b:443` – static analysis: would this connection be allowed,
  and by which rule?

## Egress gateways `[later]`
Route a set of pods' external egress through specific gateway nodes with stable source
IPs (for allow-listing by third parties). Implemented by redirecting to the gateway node's
tunnel and SNATing there.

## Open questions

- Identity allocation: central (API objects, Cilium-style CRD identities) vs
  deterministic hashing (no coordination, collision risk at 24 bits → use 32 bits + collision
  detection).
- Default-deny between tenants by default? (Baseline policy.)
- How to treat traffic from `microvm` pods – identical, since enforcement is host-side.

## Build checklist

- [ ] Policy types (3 tiers), validation, conformance tests (port Kubernetes NetworkPolicy
      e2e test cases as a semantics check).
- [ ] Identity controller + ipcache distribution.
- [ ] Policy compiler (selectors × identities → per-endpoint allow tuples) with
      incremental updates.
- [ ] BPF enforcement with verdict generation re-check.
- [ ] Audit mode and `rwctl net can`.
- [ ] FQDN policy via DNS observation.
