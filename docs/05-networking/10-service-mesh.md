# Service Mesh

**K8s analogue:** Istio (sidecar mode with Envoy; ambient mode with ztunnel + waypoint
proxies), Linkerd (Rust `linkerd2-proxy` sidecars), Cilium service mesh (per-node Envoy),
Consul Connect, SPIFFE/SPIRE.

## What meshes provide
1. **mTLS between workloads** with workload identities (zero-trust networking).
2. **L7 traffic management**: retries, timeouts, circuit breaking, traffic splitting,
   fault injection, header-based routing.
3. **L7 authorization**: allow `GET /api/*` from identity X.
4. **Telemetry**: golden metrics (rate, errors, latency) per service pair, tracing headers.

## Architectures and their costs

| Model | Example | Per-pod cost | Pros | Cons |
|---|---|---|---|---|
| Sidecar per pod | Istio classic, Linkerd | 1 proxy per pod: Envoy ~50–100 MB, linkerd2-proxy ~10–20 MB; CPU per request; +latency 2 hops | Strong isolation; per-pod config | Huge aggregate overhead; sidecar lifecycle problems; upgrades restart every pod |
| Per-node L4 + optional L7 waypoints | Istio ambient (ztunnel in Rust + Envoy waypoints) | ztunnel per node; waypoint per namespace/service when L7 needed | Much less overhead; no pod restarts to upgrade | Node proxy is shared (multi-tenant blast radius); L7 needs extra hop |
| Per-node full L7 | Cilium (Envoy per node) | One Envoy per node | Low overhead | Shared L7 proxy across tenants; Envoy config scale |
| Proxyless | gRPC xDS | 0 | Best performance | Language/library-specific |

## Rustwain approach `[proposed]`

Layered, so most users get mTLS and identity without running a "mesh":

### Layer 1: Built-in workload identity and mTLS at the node (core)
- Every pod gets a **workload identity** (SPIFFE ID:
  `spiffe://<cluster>/ns/<ns>/sa/<serviceaccount>`) with short-lived X.509 SVIDs issued by
  the cluster CA via the agent (see [identity and PKI](../09-security/01-identity-and-pki.md)).
- **Transparent mTLS** between pods (opt-in per namespace/tenant): the agent runs a
  node-level L4 proxy (like ztunnel – which is Rust!) that:
  - Intercepts outbound connections from mesh-enabled pods (redirect via socket-LB to a
    local proxy socket, or `sk_lookup`/TPROXY),
  - Establishes mTLS to the destination node's proxy using the **source pod's** identity
    (the proxy holds per-pod certificates – it must be trusted, which it is: it's part of the
    agent),
  - Destination proxy verifies, applies identity-based L4 authorization, and forwards to
    the destination pod.
  - Uses HTTP/2 CONNECT tunnelling (HBONE in Istio ambient) to multiplex connections between
    node pairs.
- Cost: one proxy per node, shared, Rust, no sidecars. Pods don't restart to enable it.
- The identity-based network policy (L3/L4) works with or without mTLS; mTLS adds
  cryptographic proof of identity rather than trusting IP → identity mapping.

### Layer 2: L7 waypoint proxies (optional workloads)
- For services that want L7 features (retries, splits, L7 authz), deploy a **waypoint**
  (the `rustwain-gateway` binary in waypoint mode) per namespace or per service; the
  node proxy routes traffic to the waypoint first.
- Configured with Gateway API (`HttpRoute` attached to Services – the GAMMA initiative's
  model for mesh).

### Layer 3: Proxyless
- The agent serves xDS (EDS/CDS/LDS minimal) for gRPC clients wanting client-side LB and
  identity-aware routing without any proxy `[later]`.

## Why not sidecars
- Aggregate overhead at 500 pods/node × 20 MB = 10 GB of RAM in proxies.
- Lifecycle complexity (mostly solved by native sidecars, but still).
- Upgrading the mesh = restarting every pod.
- Still supported for apps that need a sidecar from a third party (Rustwain doesn't
  prevent it).

## Trust and multi-tenancy concerns
- A node-level proxy holding keys for all pods on the node means a compromised node
  compromises all identities on it – but a compromised node already compromises those
  pods. The real risk is a bug in the shared proxy leaking between tenants on the same node
  → strict tenant-aware design, memory safety (Rust), and optionally per-tenant proxy
  processes.

## Open questions

- Implement the node L4 proxy ourselves (tokio + rustls + h2) vs integrate ztunnel (Rust,
  Apache-2.0, designed for Istio's xDS control plane – we'd have to serve its config
  protocol). Writing our own is very aligned with the project but large; ztunnel is a great
  reference implementation.
- HBONE compatibility with Istio for interop?

## Build checklist

- [ ] Workload identity issuance (SVIDs) via agent.
- [ ] Node L4 mTLS proxy MVP: redirect, mTLS with per-pod identity, HTTP/2 CONNECT tunnels.
- [ ] Identity-based L4 authz in proxy.
- [ ] Waypoint mode in gateway.
- [ ] Overhead benchmark vs Istio sidecar / ambient / Linkerd.
