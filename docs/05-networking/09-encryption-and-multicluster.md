# Encryption in Transit and Multi-Cluster

**K8s analogue:** Cilium/Calico WireGuard or IPsec transparent encryption, service mesh
mTLS, Multi-Cluster Services API (`ServiceExport`/`ServiceImport`), Cilium ClusterMesh,
Submariner, Skupper, KubeFed (retired).

## Transparent node-to-node encryption

### WireGuard `[proposed default when enabled]`
- In-kernel (5.6+), modern crypto (ChaCha20-Poly1305, Curve25519), simple config, fast
  (multi-Gbps per core), roams well.
- One device per node `rwwg0`; each node has a keypair (private key generated locally,
  never leaves the node); public key published in the Node object.
- Peer config: for every other node, `{pubkey, endpoint=nodeIP:51871, allowed-ips=pod
  blocks of that node}`. At 5,000 nodes = 5,000 peers per device – WireGuard handles this
  (peer lookup by allowed-ips trie) but config pushes must be incremental.
- Encrypts pod-to-pod (and optionally node-to-node host traffic).
- Key rotation: periodic new keypair; publish new public key; peers update; brief overlap
  handled by WireGuard's handshake (each handshake renegotiates session keys every 2 min
  anyway – long-term key rotation is about compromise recovery).
- Limitations: no per-workload identity (node-level), no FIPS-approved primitives (blocked
  in some regulated environments → IPsec alternative).

### IPsec `[later]`
- Kernel XFRM with AES-GCM (FIPS-capable, NIC offload available). More complex key
  management (we'd generate SAs per node pair). Option for FIPS environments.

### Node-level mTLS (workload identity) – see service mesh
WireGuard proves "this packet came from node X"; mTLS proves "this connection came from
workload identity Y". Both have a place – see [service mesh](10-service-mesh.md).

## Control-plane traffic
All control-plane connections (client↔API, agent↔API, server↔server Raft) are mTLS with
cluster PKI regardless of dataplane encryption. See
[identity and PKI](../09-security/01-identity-and-pki.md).

## Multi-cluster

Reasons people run multiple clusters: blast radius, regions, compliance, scale limits,
team autonomy. Needs: cross-cluster service discovery, cross-cluster connectivity,
consistent policy/identity, workload placement across clusters.

### Plan `[later]`
1. **Cluster identity**: each cluster has an ID and its own CA; a trust bundle federation
   (exchange CA roots) lets workloads authenticate across clusters (SPIFFE trust domain
   federation model).
2. **Connectivity**: non-overlapping pod CIDRs across clusters (validated by a "mesh"
   registry), WireGuard tunnels between gateway nodes or full mesh between nodes.
3. **Service export/import**: `ServiceExport` in cluster A; clusters in the same
   "clusterset" learn endpoints via a sync controller (each cluster's agents get remote
   endpoints in their EndpointSets with a `cluster` attribute) → global services with
   locality preference (prefer local cluster, fail over remote).
4. **Policy**: identities include cluster ID; policies can select remote cluster
   workloads.
5. **Placement**: a higher-level multi-cluster scheduler is out of scope; GitOps-style
   fan-out covers most needs.

Lessons from others:
- KubeFed (federating every API type) failed due to complexity – don't federate the whole
  API; federate *specific* concerns (services, identity).
- Cilium ClusterMesh: each cluster's agents connect to remote clusters' state (etcd) –
  simple but tight coupling.
- MCS API (ServiceExport/ServiceImport) is the right minimal abstraction.

## Open questions

- Gateway-node tunnels vs full node mesh across clusters (full mesh = N×M peers).
- IPsec priority (FIPS demand).

## Build checklist

- [ ] WireGuard device management via generic netlink; incremental peer updates.
- [ ] Node key generation, publication, rotation.
- [ ] Encryption on/off per cluster with live migration (rolling enablement without
      dropping traffic: accept both plain and encrypted during transition).
- [ ] Multi-cluster: design spike only.
