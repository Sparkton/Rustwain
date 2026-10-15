# Ingress and Gateway

**K8s analogue:** Ingress (+ ingress-nginx, Traefik, HAProxy controllers), Gateway API
(GatewayClass, Gateway, HTTPRoute, GRPCRoute, TLSRoute, TCPRoute), cert-manager.

## Context

- **Ingress** (Kubernetes) is underspecified: host/path routing + TLS only; everything else
  is controller-specific annotations (`nginx.ingress.kubernetes.io/...`) – non-portable,
  and annotation injection caused serious CVEs (ingress-nginx "IngressNightmare",
  CVE-2025-1974: unauthenticated RCE via the admission webhook's config validation).
  ingress-nginx is being retired (announced 2025, maintenance ending 2026).
- **Gateway API** fixes this with role-oriented resources (infrastructure provider →
  GatewayClass, cluster operator → Gateway, app dev → Routes), typed matching
  (headers, query, method), traffic splitting, cross-namespace references with explicit
  `ReferenceGrant`. It's the good design.

## Rustwain approach `[proposed]`

- Adopt the **Gateway API model** natively (resource shapes close to upstream – it's a
  well-reviewed API): `GatewayClass`, `Gateway`, `HttpRoute`, `GrpcRoute`, `TlsRoute`,
  `TcpRoute`, `ReferenceGrant`.
- Ingress objects supported only via the Kubernetes import adapter (translated to
  Gateway + HttpRoute).
- Built-in gateway implementation: **`rustwain-gateway`**, a Rust L7 proxy deployed as a
  workload (DaemonSet or Deployment behind a LoadBalancer service).
  - Built on `hyper` + `rustls` (+ `tokio`), or on Cloudflare's **Pingora** framework
    (Rust, production-proven at Cloudflare scale, HTTP/1/2, connection pooling, load
    balancing). Pingora is a strong candidate. `[open]`
  - Config delivered by watching Gateway API objects directly (no config-file
    reloads → no nginx reload storms causing dropped connections at scale).
  - Features: host/path/header/method matching, weighted backends (canary), retries with
    budgets, timeouts, header manipulation, redirects, mirroring, gRPC, WebSockets, HTTP/3
    `[later]` (via `quinn`/`h3`).
  - Backends addressed as Services → resolved to EndpointSets directly (no extra kube-proxy
    hop; gateway picks the pod).
- Pluggable: any third-party Gateway API implementation (Envoy Gateway, etc.) could run
  against the adapter `[open]`.

## TLS certificates
- Gateway listeners reference Secrets with certs.
- Built-in **ACME client** `[proposed]` (cert-manager equivalent, scoped): a
  `Certificate` object with issuers (ACME HTTP-01 via the gateway itself, DNS-01 via
  provider plugins, or internal cluster CA). Rust crate `instant-acme`.
- Automatic renewal, stored as Secrets, hot-reloaded by the gateway.

## North-south traffic path

```
client → (cloud LB or bare-metal LB VIP) → node (XDP/TC LoadBalancer service)
       → gateway pod (TLS termination, routing) → backend pod (direct to pod IP)
```

With `externalPolicy: Local` on the gateway's Service, client IPs are preserved to the
gateway, which forwards them as `X-Forwarded-For` / PROXY protocol.

## Open questions

- Pingora vs hyper-based custom proxy.
- Should the gateway run inside the agent (node-level, like Cilium's per-node Envoy)
  rather than as a workload? Isolation and scaling argue for workload; resource efficiency
  argues for node-level. Lean: workload.

## Build checklist

- [ ] Gateway API types (subset: Gateway, HttpRoute, GrpcRoute, ReferenceGrant).
- [ ] `rustwain-gateway` MVP: HTTP/1.1+2, TLS (rustls), routing, weighted backends.
- [ ] ACME `Certificate` controller.
- [ ] Ingress → Gateway translation in import adapter.
