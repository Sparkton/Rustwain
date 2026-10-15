# DNS and Service Discovery

**K8s analogue:** CoreDNS (cluster DNS), NodeLocal DNSCache, kubelet-generated
`resolv.conf` (`ndots:5`, search domains), DNS for Services and Pods.

## Kubernetes DNS and its problems

- CoreDNS runs as a Deployment (typically 2 replicas) behind a ClusterIP. Every pod's
  `resolv.conf` points to that VIP.
- `resolv.conf` in pods:
  ```
  nameserver 10.96.0.10
  search <ns>.svc.cluster.local svc.cluster.local cluster.local <host search domains>
  options ndots:5
  ```
- **`ndots:5` amplification**: any name with fewer than 5 dots is first tried with each
  search domain. `api.example.com` (2 dots) → queries
  `api.example.com.<ns>.svc.cluster.local`, `api.example.com.svc.cluster.local`,
  `api.example.com.cluster.local`, (host domains…), then `api.example.com` – each for A
  *and* AAAA → **8–10 queries per external lookup**, most returning NXDOMAIN.
- **conntrack races on UDP DNS**: glibc sends A and AAAA queries in parallel from the same
  socket; with iptables DNAT to the DNS VIP, both packets race to create a conntrack
  entry → one gets dropped → 5-second timeouts (the infamous "DNS 5s delay" bug; fixed
  partially in kernels 5.0+, mitigated with `single-request-reopen`, or NodeLocal
  DNSCache).
- CoreDNS is a central bottleneck; NodeLocal DNSCache (a DaemonSet with link-local IP +
  iptables NOTRACK) was added to fix latency and conntrack issues.
- musl (Alpine) resolver behaves differently from glibc (parallel queries to all
  nameservers, no `single-request` options, historically no TCP fallback for large
  responses until musl 1.2.4).

## Rustwain DNS design

### Node-local resolver in the agent `[decided]`
- A DNS server built into the agent (separate sandboxed child process for blast radius
  `[proposed]`), listening on a **link-local address inside every pod's routing reach**
  (e.g. `169.254.20.10` / `fd00::53` routed to the host) – or more simply, the socket-LB
  `sendmsg` hook steers pods' DNS queries to the local resolver.
- Answers cluster names **from local memory**: the agent already watches services and
  endpoints (for the dataplane) – it can answer `svc.cluster.local` queries without any
  network hop. No CoreDNS deployment at all.
- Forwards external names to upstream resolvers (from host config, with systemd-resolved
  awareness – see [host variance](../01-linux-foundations/05-linux-host-variance.md)),
  with a shared cache, prefetching of popular names, and serve-stale on upstream failure.
- Library: `hickory-dns` (formerly trust-dns; Rust) for protocol parsing and the
  resolver/forwarder pieces. Our authoritative zone logic is custom (computed from
  service/endpoint maps).

### Fixing ndots
Options:
1. Default `ndots:1` `[proposed]` with search domains `<ns>.svc.<cluster>`,
   `svc.<cluster>`. Short service names (`api`, `api.shop`) still work (0 or 1 dot);
   external names (`api.example.com`) resolve in one query. Breaks only `a.b.svc`-style
   partially qualified names with ≥ 1 dot... `api.shop` has 1 dot → with ndots:1 it's
   tried as absolute first (NXDOMAIN upstream – unless the resolver short-circuits) then
   with search. To avoid leaking internal names to upstream:
2. **Resolver-side search handling**: the local resolver knows the cluster zone; when it
   receives `api.shop.` (absolute) and `shop` is a namespace in the cluster, it can answer
   NXDOMAIN fast without going upstream, and the client then tries the search path. Or
   **autopath** (CoreDNS plugin): on the first query `api.example.com.<ns>.svc.cluster.local`,
   the server recognises the search-path pattern and answers directly with the
   final result for `api.example.com` (with a CNAME), eliminating the remaining queries.
   Autopath needs to know the client pod's namespace → we know it from the source IP.
3. Combined proposal: keep `ndots:5` for compatibility **but** implement autopath and
   negative caching in the node-local resolver, so the amplification never leaves the
   node and costs microseconds. Users can opt into `ndots:1`.

Decision `[proposed]`: option 3 (compat-first, fixes cost) with per-pod `dnsConfig`
override.

### No conntrack races
The query never crosses a NAT: either the pod's resolver address is directly the local
resolver (routed link-local, no DNAT) or socket-LB rewrites at `sendmsg` (no conntrack
entry). The 5-second race cannot occur.

### Records served
| Name | Records |
|---|---|
| `<svc>.<ns>.svc.<zone>` | A/AAAA → ClusterIP(s); headless → ready pod IPs |
| `_<port>._<proto>.<svc>.<ns>.svc.<zone>` | SRV |
| `<pod-hostname>.<subdomain>.<ns>.svc.<zone>` | A/AAAA (StatefulSet pods) |
| `<a-b-c-d>.<ns>.pod.<zone>` | Pod IP-derived names (Kubernetes legacy, insecure – disabled by default) |
| PTR for pod and service IPs | Reverse lookups |
| ExternalName services | CNAME |

Cluster zone default `cluster.local` (compat), configurable.

### Caching and TTLs
- Cluster records: TTL 5 s (configurable); but since the resolver is local and
  watch-driven, it's always fresh – the TTL only matters for client-side caches (glibc
  doesn't cache; JVM does!).
- Negative caching for upstream NXDOMAIN per RFC 2308.
- Cache size bounded (LRU), per-pod query rate limits to stop a misbehaving pod from
  hammering upstreams.

### DNS for microVM pods
The guest's resolver points to the pod's gateway/link-local resolver address – reachable
through the TAP, served by the same node-local resolver.

## Beyond DNS: discovery API

DNS is lossy (no ports without SRV, no health detail, client caching). For clients that
want rich discovery:
- Pods can read their **own namespace's endpoints** through a local, rate-limited
  read-only endpoint (`http://169.254.20.10/v1/endpoints/<svc>`) `[open]` – useful for
  client-side load balancing (gRPC). gRPC's xDS-based proxyless mode is the standard for
  this; serving a minimal **xDS** (EDS/CDS) from the agent `[later]` would let gRPC clients
  do client-side LB natively.

## Open questions

- Separate DNS child process vs in-agent task.
- Expose xDS from the agent?
- Default `ndots` – compat vs correctness.

## Build checklist

- [ ] Node-local resolver (hickory-dns based) with cluster zone from watch state.
- [ ] Autopath + negative cache + serve-stale.
- [ ] systemd-resolved-aware upstream discovery.
- [ ] Per-pod resolv.conf generation (dnsPolicy/dnsConfig equivalents).
- [ ] Benchmark: external lookup latency with ndots:5 vs CoreDNS+NodeLocal baseline.
