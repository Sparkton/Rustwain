# State Store

**K8s analogue:** etcd (Raft-replicated MVCC key-value store with watch), and the
apiserver's storage layer (etcd3 storage interface, compaction, encryption at rest).

## What Kubernetes needs from etcd

- Linearizable reads and writes.
- **MVCC**: every write creates a new revision; reads at a past revision; range queries.
- **Watch** from a revision: stream of changes for a key prefix, resumable.
- **Transactions**: compare-and-swap on `mod_revision` (optimistic concurrency).
- **Leases**: keys that expire (used for events TTL, leader election in older versions).
- **Compaction**: drop history older than N (apiserver compacts every 5 min).

## etcd pain points (why we're building our own)
- Default backend quota **2 GiB** (max recommended **8 GiB**); exceeding → cluster goes
  read-only (`mvcc: database space exceeded`) until compaction + **defragmentation**
  (defrag blocks the member for seconds-minutes).
- Request size limit 1.5 MiB.
- bbolt B+tree with a single writer and page-level CoW: write amplification; free pages
  not returned to the OS without defrag.
- Separate operational component: its own TLS, backups, upgrades, monitoring, disk
  latency sensitivity (`fsync` latency → leader elections; "etcdserver: request timed out").
- Watch fan-out happens in apiserver anyway (watch cache), duplicating state in memory.
- Events storms (Events are just objects in etcd) can overwhelm it – many clusters run a
  separate etcd for events.

See [storage and etcd pros/cons](../12-critique/04-storage-and-etcd-pros-cons.md).

## Rustwain store: embedded, Raft-replicated, MVCC

Decision `[proposed]`: **the store is a library embedded in `rustwain server`**:
- Raft consensus via **openraft** (async Rust Raft implementation, used by Databend and
  others; supports joint consensus membership changes, snapshots, learners, leader
  lease reads) – or `raft-rs` (TiKV's, synchronous, very battle-tested). Lean: openraft
  (async/tokio-native), keep the Raft layer behind our own trait so we can swap.
- **Raft log storage**: a dedicated append-only segmented log file (we write it; it's a
  simple and performance-critical component) with `fdatasync` batching (group commit).
- **State machine storage**: an embedded KV engine holding the MVCC keyspace.

### Storage engine options
| Engine | Type | Pros | Cons |
|---|---|---|---|
| **redb** | Copy-on-write B-tree, pure Rust | Simple, ACID, MVCC-friendly reads, stable file format | Single writer; CoW write amp similar to bbolt (but with better free-space reuse) |
| **fjall** | LSM-tree, pure Rust | High write throughput, compaction reclaims space automatically, prefix scans | Younger project; LSM read amplification |
| RocksDB (rust-rocksdb) | LSM, C++ | Industry standard (TiKV, CockroachDB earlier) | C++ dependency, build complexity – violates Rust-only spirit |
| sled | Rust | – | Unmaintained-ish/beta for years; avoid |
| Custom in-memory + snapshot | — | Fastest; whole dataset in RAM (k8s data is usually < few GB) | Memory bound; restart time = snapshot load + log replay |

Interesting observation: Kubernetes' API server **already holds essentially the entire
dataset in memory** (watch cache). etcd holds it on disk with history. So the "real"
design for an embedded store is:

- **In-memory MVCC index** (the authoritative current state + bounded history) – this
  *is* the watch cache.
- **Raft log** for durability.
- **Periodic snapshots** of the in-memory state to disk (for log truncation and fast
  restart), written incrementally.

Decision `[proposed]`: **in-memory state machine + Raft log + snapshots**, with an
on-disk engine (redb/fjall) only as a *snapshot format* option. Bound: dataset must fit in
RAM (we cap total store size, default 8 GiB, and enforce per-namespace/tenant quotas on
object count/bytes). This collapses etcd + watch cache into one data structure and
removes the double memory cost.

### Data model
```
key:   /<group>/<resource>/<namespace>/<name>      (cluster-scoped: no namespace segment)
value: encoded object bytes
meta:  create_revision, mod_revision, version (per-key modification count), lease_id?
```
- Global monotonically increasing **revision** = Raft log index of the committing entry
  (one revision per Raft entry; a batch of writes in one entry shares it – or use
  `(index, sub-index)` encoded into one u64 `[open]`).
- MVCC history: per key, a small list of `(revision, value | tombstone)` versions retained
  within the **history window** (time- or revision-bounded, e.g. last 10 minutes or 100k
  revisions), then compacted. Persistent structure (`im`-style HAMT/B-tree with
  structural sharing) lets readers hold a consistent snapshot at revision N cheaply.

### Operations (state machine commands)
```rust
enum Command {
    Txn { compares: Vec<Compare>, success: Vec<Op>, failure: Vec<Op> }, // etcd-style
    LeaseGrant { id, ttl }, LeaseRevoke { id }, // persisted leases (rare use; see node leases)
    Compact { revision },
}
enum Op { Put { key, value, lease }, Delete { key }, DeleteRange { prefix } }
enum Compare { ModRevision(key, Cmp, u64), Exists(key), Version(key, Cmp, u64) }
```
Validation, defaulting and admission all happen **before** proposing to Raft (in the API
layer), so the state machine stays simple and deterministic.

### Indexes
Secondary indexes maintained in the state machine (deterministic, rebuilt on snapshot
load):
- By namespace (prefix – free).
- By label (`label key=value` → set of keys) – enables server-side label selection
  without scanning.
- By configured fields (`spec.node` for pods, `involvedObject` for events, owner
  references) – powers field selectors and node-scoped watches efficiently.

### Watch
- Every applied entry produces change events `(revision, key, prev_value, new_value)`.
- Watchers subscribe with `(prefix | index predicate, from_revision)`.
- If `from_revision` is within the history window: replay from history then live.
  Otherwise: `410 Gone`-equivalent `TooOld` → client relists.
- Fan-out: see [watch and caching](04-watch-and-caching.md).

### Durability and performance
- Write path latency = network RTT to quorum + `fdatasync` on leader and followers.
  **Group commit**: batch many proposals into one Raft append + one fsync – throughput
  scales with batching.
- Target: 10k writes/s sustained on NVMe with 3 replicas, p99 commit < 25 ms.
- Disk: Raft log on fast storage; snapshots can be on slower disk.
- Snapshot every N entries (e.g. 100k) or M MB of log; followers far behind receive
  snapshots via InstallSnapshot (streamed, chunked).

### Leases and time
- Raft leader **lease reads** (leader serves reads without a quorum round trip while its
  lease is valid) depend on bounded clock drift; openraft supports leader lease. We use
  ReadIndex by default (safe without clock assumptions) and lease reads optionally.
- Node heartbeats: leader-local, non-persisted (see
  [registration](../04-node-agent/05-node-registration-and-heartbeat.md)).

### Membership changes
- Joint consensus (openraft) for adding/removing servers safely.
- `rwctl server add/remove`; learners first (catch up), then promote to voter.
- Avoid even member counts; 3 or 5 voters recommended; learners for read scale.

### Encryption at rest
- Values for sensitive kinds (Secrets) encrypted with AES-GCM using a DEK, wrapped by a KEK
  (local keyfile, TPM, or external KMS). Applied in the API layer before proposing, so the
  Raft log and snapshots contain only ciphertext. See [secrets](../09-security/03-secrets.md).
- Optionally encrypt the entire log/snapshot files (full-disk encryption is often simpler).

### Backup and restore
- Online snapshot export: `rwctl store snapshot save` streams a consistent snapshot at
  revision N (from any member). Restore bootstraps a new single-member cluster from it.
  See [backup and DR](../10-cluster-operations/07-backup-and-disaster-recovery.md).

### Separate keyspaces
- **Events** go to a separate, non-replicated-or-lightly-replicated ring buffer store per
  server `[proposed]` (events are best-effort, high-volume, short-lived) – keeps the main
  Raft log clean. See [audit and events](../10-cluster-operations/10-audit-and-events.md).

## Correctness strategy
Consensus code must be tested ruthlessly:
- **Deterministic simulation testing** (FoundationDB/TigerBeetle style): run the full
  server cluster in a single-threaded simulator with injected network partitions, message
  reordering, disk faults (torn writes, fsync failures), and clock skew; check
  linearizability with a checker (e.g. port of `porcupine` or Knossos-style model
  checking). Rust tools: `madsim`, `turmoil` (tokio-based network simulation), or our own
  sim harness.
- **Jepsen-style** tests against real binaries in VMs.

## Open questions

- openraft vs raft-rs.
- Pure in-memory + snapshots vs disk-backed engine (dataset size bound acceptable?).
- One revision per entry vs per write in batch.
- Should we offer an "etcd v3 API compatibility" server? (Would let Kubernetes itself use
  our store – a neat milestone: run real kube-apiserver on top of Rustwain store, like
  kine does for SQL databases.) Interesting as a test of correctness; `[open]`.

## Build checklist

- [ ] `rustwain-store` crate: Raft log, state machine, MVCC index, snapshots.
- [ ] Txn/compare semantics + watch from revision.
- [ ] Label and field indexes.
- [ ] Deterministic simulation test harness with linearizability checker.
- [ ] Benchmarks: write throughput/latency, watch fan-out, restart time at 2/8 GiB.
- [ ] (Optional) etcd v3 gRPC compatibility shim for testing with kube-apiserver.
