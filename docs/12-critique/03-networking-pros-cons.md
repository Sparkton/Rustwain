# Networking Stacks: Pros and Cons

## kube-proxy

| Mode | Pros | Cons |
|---|---|---|
| iptables (long-time default) | Works everywhere; well understood | O(n) rule updates (full `iptables-restore`), O(n) first-packet matching; minutes to sync at 10k+ services; conntrack table pressure; `iptables-legacy` vs `-nft` confusion |
| IPVS | O(1) lookups; more LB algorithms | Still needs iptables for some features; IPVS module quirks; graceful termination edge cases; being de-emphasised |
| nftables (GA 1.33) | Incremental updates; sets/maps; modern kernel API | Still netfilter + conntrack; newer, less battle-tested |
| (eBPF replacements: Cilium, Calico eBPF) | O(1), socket-LB, no conntrack for services | Requires modern kernels; separate project |

General kube-proxy issues:
- Services implemented as per-packet DNAT → conntrack required → `nf_conntrack: table full`,
  UDP conntrack races (DNS 5 s timeouts), stale UDP entries after endpoint changes.
- Sync latency at scale → traffic to terminated pods.
- No topology/locality beyond recent traffic distribution features.

## CNI plugins

### Flannel
| Pros | Cons |
|---|---|
| Very simple; VXLAN overlay just works | No network policy (needs Calico in "Canal") |
| Small footprint | Limited features; per-node /24 IPAM |

### Calico
| Pros | Cons |
|---|---|
| Mature; BGP-native routing (no overlay needed on suitable networks) | Many modes (iptables, eBPF, VPP, Windows) – configuration complexity |
| Rich policy (GlobalNetworkPolicy, tiers in enterprise) | Typha/Felix/BIRD/confd components |
| IPAM with blocks (borrowing, multiple pools) | Datastore choices (k8s API vs etcd) |
| eBPF dataplane option | |

### Cilium
| Pros | Cons |
|---|---|
| eBPF datapath: kube-proxy replacement, socket-LB, identity-based policy, L7 via Envoy, Hubble observability, ClusterMesh, WireGuard/IPsec, bandwidth manager (EDT), BGP, netkit | Complexity: many features and knobs; large agent memory (100s of MB at scale) |
| Excellent performance | Kernel requirements; BPF debugging is hard |
| Strong community (CNCF graduated) | Identity allocation via CRDs can churn at scale; per-node Envoy is a shared L7 proxy |
| | Upgrades require care (map migrations) |

### AWS VPC CNI / Azure CNI / GKE native
| Pros | Cons |
|---|---|
| Pods get VPC IPs: no overlay, native security groups | IP exhaustion in subnets; ENI/IP limits cap pods per node (e.g. small instances ~10–30 pods) |
| Native cloud LB IP targets | Cloud-specific |

## CoreDNS
| Pros | Cons |
|---|---|
| Plugin architecture; flexible | Central bottleneck behind a ClusterIP |
| autopath, caching plugins | ndots:5 amplification by default; conntrack races; NodeLocal DNSCache needed for scale |

## Service meshes
| Mesh | Pros | Cons |
|---|---|---|
| Istio (sidecar) | Full-featured L7, mTLS, policy | Heavy: Envoy per pod; complexity; upgrades restart pods |
| Istio ambient | No sidecars; ztunnel (Rust) per node; waypoints for L7 | Newer; shared node proxy |
| Linkerd | Simple, Rust micro-proxy, low overhead | Sidecars; fewer L7 features; licensing changes for stable builds (2024) |
| Cilium mesh | No sidecars; eBPF + per-node Envoy | Shared L7 proxy; tight coupling to Cilium |

## Ingress controllers
| Controller | Pros | Cons |
|---|---|---|
| ingress-nginx | Ubiquitous | Annotation sprawl; reload-based config; IngressNightmare CVE-2025-1974; retirement announced |
| Traefik / HAProxy / Contour / Envoy Gateway | Dynamic config; Gateway API support | Each has its own extensions |
| Gateway API (spec) | Role-oriented, typed, portable | Still maturing for some route types; implementations vary |

## Network policy
| | Pros | Cons |
|---|---|---|
| NetworkPolicy API | Portable baseline | No deny, no cluster-wide, no L7, no FQDN, no logging; implementation-dependent edge cases |
| AdminNetworkPolicy | Tiers, deny, cluster scope | New; limited implementations |

## Takeaways for Rustwain
- eBPF-first with socket-LB (no service conntrack), identity-based policy, netkit.
- Node-local DNS in the agent with autopath; no central DNS bottleneck.
- Built-in WireGuard; sidecar-less mTLS at the node.
- Gateway API model natively; no annotation sprawl.
- nftables fallback for older kernels – acknowledge it's second-class.
- Avoid Cilium's main pain (complexity, memory) by scoping features tightly and making
  identity allocation cheap.
