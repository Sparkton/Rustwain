# IPAM and Dual Stack

**K8s analogue:** `--cluster-cidr`, `--service-cluster-ip-range`, node `podCIDR(s)`
allocated by kube-controller-manager's node IPAM, host-local IPAM plugin, Calico IPAM
blocks, Cilium cluster-pool/multi-pool IPAM, AWS VPC CNI ENI IPs, dual-stack (GA 1.23).

## Address spaces

| Space | Example (v6 / v4) | Allocated by |
|---|---|---|
| Pod network | `fd00:10:42::/48` / `10.42.0.0/16` | IPAM controller → node blocks; agent → pods |
| Service VIPs | `fd00:10:96::/112` / `10.96.0.0/16` | API server at Service create |
| Node IPs | from infrastructure | DHCP/static/cloud |
| LoadBalancer pools | e.g. `203.0.113.0/28` | LB controller |

Validation at cluster init: no overlaps between pod, service, node and LB ranges, and
no overlap with common host networks (`docker0` 172.17/16, typical home 192.168/16) –
warn.

## Two-level allocation `[decided]`

1. **Cluster → node blocks** (control plane, rare): IPAM controller assigns blocks to nodes
   from the pod CIDR. Stored as `IpBlock` objects (or in Node status).
2. **Node block → pod IP** (agent, frequent, local): agent allocates from its blocks with a
   local bitmap persisted in its checkpoint DB. **No API round trip per pod.**

Blocks:
- IPv6: one **/64 per node** by default (2^64 addresses – never runs out; allows
  stateless mapping tricks).
- IPv4: **/26 blocks** (64 addrs) allocated on demand, multiple per node; released when
  empty for a while. Avoids Kubernetes' fixed-per-node `/24` that wastes space on small
  nodes and caps dense nodes at 254 pods (Kubernetes default max-pods 110 partly derives
  from this).

Routing implications: native routing with many blocks per node = more routes (fine with
BPF LPM trie; for BGP, aggregate where possible).

## IP lifecycle details
- Allocation keyed by pod UID; idempotent (agent restart returns the same IP).
- **Quarantine** after release (default 30 s) before reuse – prevents a new pod receiving
  traffic intended for the old pod (stale conntrack, stale endpoint caches on other
  nodes).
- **Static/sticky IPs** for StatefulSet pods or VMs `[proposed]`: an `IpClaim` bound to
  a pod identity (e.g. `db-0`) – survives rescheduling *within* the same IP pool. With
  node blocks, a sticky IP may come from another node's block → requires a /128 or /32
  route exception (ipcache entry) – supported by the BPF LPM design.
- Live-migrated VMs keep their IP the same way.

## API sketch: the allocators

Two traits, one per level. The cluster-level one is implemented by the IPAM controller
(and by cloud plugins); the node-level one lives in the agent and never talks to the API
on the pod-creation path.

```rust
/// Cluster level: hands out blocks to nodes. Implementations: `PoolIpam` (built-in,
/// blocks from the cluster CIDR stored as IpBlock objects), `AwsPrefixDelegation`, …
#[async_trait]
pub trait IpamProvider: Send + Sync {
    async fn request_block(&self, node: &NodeName, pool: &PoolName, family: Family)
        -> Result<IpBlock, IpamError>;               // PoolExhausted, NotPermitted, …
    async fn release_block(&self, node: &NodeName, block: &IpBlockId) -> Result<(), IpamError>;
    /// Sticky IPs that must be routable on `node` as /128 or /32 exceptions.
    async fn claim(&self, claim: &IpClaimRef, node: &NodeName) -> Result<IpAddr, IpamError>;
}

pub struct IpBlock {
    pub id: IpBlockId,
    pub cidr: IpNet,                // /64 (v6) or /26 (v4)
    pub pool: PoolName,
    pub node: NodeName,
    pub generation: u64,            // fencing: agent writes include the block generation
}

/// Node level: synchronous, in-memory, checkpointed. Called from the pod's prepare step.
pub trait PodIpAllocator {
    /// Idempotent: the same `uid` always gets the same addresses while it holds them.
    fn allocate(&mut self, uid: PodUid, req: &IpRequest, now: WallTime)
        -> Result<PodIps, AllocError>;
    /// Moves addresses to quarantine; they become allocatable at `now + quarantine`.
    fn release(&mut self, uid: PodUid, now: WallTime);
    /// Blocks that have been empty (incl. quarantine) for `idle_for`, for return.
    fn reclaimable_blocks(&self, now: WallTime, idle_for: Duration) -> Vec<IpBlockId>;
    /// Free addresses remaining across blocks of `family` in `pool`.
    fn headroom(&self, pool: &PoolName, family: Family) -> u64;
}

pub struct IpRequest {
    pub pool: PoolName,
    pub families: FamilySet,        // {V6}, {V4}, {V6, V4}
    pub sticky: Option<IpClaimRef>, // resolved by the controller before the pod arrives
}

pub enum AllocError {
    /// No free address in local blocks. The prepare step parks the pod with
    /// `blocking: IpPending` and the background refiller is already asking for a block.
    NeedBlock { pool: PoolName, family: Family },
    StickyNotRouted(IpClaimRef),
}

/// The default implementation.
pub struct BitmapAllocator {
    blocks: BTreeMap<IpBlockId, BlockState>,
    by_uid: HashMap<PodUid, PodIps>,            // idempotency
    db: CheckpointHandle,                       // redb table "ipam" — see schema below
}

struct BlockState {
    block: IpBlock,
    used: BitVec,                    // v4 /26: 64 bits; v6 /64: sparse (see note)
    next_hint: u32,                  // rotate allocations → spreads reuse across the block
    quarantine: VecDeque<(u32 /*offset*/, WallTime /*until*/)>,
    empty_since: Option<WallTime>,
}
```

Notes on the sketch:

- **IPv6 /64 is not a 2^64-bit bitmap.** v6 blocks allocate from a dense 2^16 sub-range
  (`<block>::<agent-epoch-salt>:0000/112`) with a 64 Kbit bitmap (8 KiB) and switch to a
  fresh sub-range when it fills. Address *count* is never the constraint on v6.
- **Reserved offsets:** v4 `.0` of each /26 is never allocated (some tools treat it as a
  network address even on routed /26s); the node-side gateway uses a link-local address
  (`169.254.1.1` / `fe80::1`), so no other offsets are reserved `[open]` – confirm no
  dataplane path needs an in-block gateway address.
- **Checkpoint schema** (`agent.db`, table `ipam`): `uid → PodIps`, `block_id → {cidr,
  generation, used bitmap, quarantine list}`. Allocation sets the bit and inserts the
  `uid` row in **one transaction**, before the address is programmed anywhere (the
  "intent before effect" rule in [pod lifecycle](../04-node-agent/02-pod-lifecycle.md)).
- **Refill ahead of need:** a background task requests a new block when `headroom` drops
  below the low-water mark (default: 8 v4 addresses, or 25% of the node's recent
  5-minute pod-creation peak, whichever is larger), so `NeedBlock` is rare on the hot
  path.
- **Leak repair:** on agent start, any `uid` row whose pod is neither in the checkpoint
  nor running on the host is released into quarantine (orphan GC). This is the fix for
  the classic host-local IPAM leak (`PP-6D`).
- **Fencing:** a block reassigned away from a node (node deleted then re-joined)
  carries a new `generation`; the agent drops any local block whose generation is stale
  after it syncs, so two nodes can never hand out addresses from the same block.

## Dual stack

- Default for new clusters `[proposed]`: **dual-stack, IPv6 primary**. Pods get one v6 and
  one v4 address. Services get both families (`ipFamilies: [IPv6, IPv4]`) by default.
- **Single-stack IPv6** supported fully: requires NAT64/DNS64 for v4-only external
  destinations – optional built-in NAT64 in the BPF dataplane `[later]` (Cilium has this).
- **Single-stack IPv4** supported for legacy.

Why IPv6 first:
- No address exhaustion; no per-node IPv4 block juggling.
- Pod IPs can be globally routable (GUA from provider prefix) → no egress NAT.
- Dual-stack in Kubernetes is often an afterthought; designing v6-first avoids v4
  assumptions (e.g. 32-bit IP map keys) baked into the dataplane. Every BPF map key uses
  a 16-byte address (v4 as v4-mapped v6) `[proposed]` – simplicity over a few bytes.

Kernel/host considerations:
- IPv6 disabled on host (`ipv6.disable=1` on cmdline, or
  `net.ipv6.conf.all.disable_ipv6=1`) – probe detects and falls back to v4-only.
- DAD (duplicate address detection) delays address usability ~1 s → disable DAD in pod
  netns (we control the addressing; duplicates are impossible by construction).
- Router advertisements: pods don't need SLAAC; static config.

## Cloud IPAM modes `[later]`
Pods with VPC-routable IPs (AWS ENI secondary IPs / prefix delegation, Azure, GCP alias
IPs) – simplify routing and security groups but cause IP exhaustion in VPC subnets (the
famous EKS problem). Provided by cloud plugins via an `IpamProvider` trait: allocate
block (prefix delegation /28 on AWS) for node.

## Multi-pool IPAM `[later]`
Different pools for different workloads (e.g. pods in tenant X get IPs from a pool routable
to their on-prem network). Pool selection via pod annotation/tenant policy; node gets
blocks from multiple pools on demand.

## Open questions

- Store blocks as separate objects (scales: one object per block) vs Node status field.
- v4-mapped v6 keys everywhere – verify BPF map memory impact at scale (16 vs 4 bytes per
  key – ~negligible relative to values).

## Build checklist

- [ ] CIDR config validation (overlaps, sizes).
- [ ] IPAM controller: block allocation, release of empty blocks, pool support.
- [ ] Agent bitmap allocator with quarantine and checkpointing.
- [ ] Dual-stack throughout types (`PodIps { v6: Option<Ipv6Addr>, v4: Option<Ipv4Addr> }`).
- [ ] Sticky IP claims.
