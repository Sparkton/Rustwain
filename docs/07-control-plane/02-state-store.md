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
RAM (we cap **live object bytes**, default 4 GiB, and enforce per-namespace/tenant quotas
on object count/bytes). This collapses etcd + watch cache into one data structure and
removes the double memory cost.

Sizing check (5k nodes / 150k pods, the [budget](../11-efficiency/01-performance-budgets.md)
target of < 8 GB RSS per server): live objects ≈ 1–2 GiB encoded (pods dominate at
~4–8 KiB each), × ~1.5 for persistent-map nodes, `Arc` headers and indexes ≈ 3 GiB, plus
the in-memory history window (bounded by bytes, below), API buffers and watch queues.
The live-bytes cap must therefore stay well below the RSS target; an earlier draft used
8 GiB for both, which cannot hold.

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
  within the **in-memory history window**, then compacted. The window is bounded by
  **bytes** (default 512 MiB), not only by time or revision count. A time bound alone
  doesn't work at the write target: 10k writes/s × ~4 KiB × 10 minutes ≈ 24 GB.
  Persistent structure (`imbl`-style HAMT/B-tree with structural sharing; `im` itself is
  unmaintained, RUSTSEC-2026-0248) lets readers hold a consistent snapshot at revision N
  cheaply.
- **Change journal** `[proposed]`: older history is served from disk. The apply loop
  appends one compact record per revision `(rev, key, new value | tombstone,
  prev_mod_revision, prev index values)` to a segmented append-only file, fsynced lazily
  (it is rebuildable from the Raft log). Watch resumes older than the in-memory window
  but newer than the journal's retention (default ≥ 15 min) replay sequentially from the
  journal. `prev index values` (labels and indexed fields of the previous version) let
  the watch hub compute "left the filter" events without loading the previous object.
  This keeps resume-after-disconnect working during large rollouts, exactly when the
  in-memory window turns over fastest.

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
- If `from_revision` is within the in-memory window: replay from memory then live. If it
  is within the change journal: replay from disk, then memory, then live. Otherwise:
  `410 Gone`-equivalent `TooOld` → client re-runs watch-list.
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
  lease is valid) depend on bounded clock drift. openraft exposes both through
  `ensure_linearizable(ReadPolicy::ReadIndex | ReadPolicy::LeaseRead)` (0.10 API; 0.9
  takes no policy argument). We use ReadIndex by default (safe without clock
  assumptions) and lease reads optionally. Note: openraft 0.10 is still an alpha series
  at the time of writing. Pin one line and keep it behind our own `Consensus` trait.
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

## Worked walkthroughs

The sections above describe the store's parts. This section strings them together into
end-to-end sequences, with the invariants each step relies on. Setup throughout: three
voters S1, S2, S3; S1 is leader in term 7; a client (controller or `rwctl`) is connected
to S2's API layer.

Notation: `idx` = Raft log index, `commit` = commit index, `applied` = last index applied
to the in-memory MVCC state machine, `rev` = store revision (= `idx` of the committing
entry, see [data model](#data-model)).

### 1. Write path: `update Deployment web` with optimistic concurrency

```
 Client        S2 (follower, API)          S1 (leader)                 S3 (follower)
   │                 │                          │                           │
   │ PUT web         │                          │                           │
   │ (rv=1041)       │                          │                           │
   │────────────────▶│                          │                           │
   │                 │ authn/authz, decode,     │                           │
   │                 │ default, validate,       │                           │
   │                 │ admission (CEL), encrypt │                           │
   │                 │ if Secret                │                           │
   │                 │                          │                           │
   │                 │ build Txn:               │                           │
   │                 │  if ModRevision(k)==1041 │                           │
   │                 │  then Put(k, bytes)      │                           │
   │                 │  else Get(k)             │                           │
   │                 │ ForwardProposal(txn) ───▶│                           │
   │                 │                          │ batcher: collect ≤ 2 ms   │
   │                 │                          │ or ≤ 256 proposals        │
   │                 │                          │ append idx 1100..1163     │
   │                 │                          │ to log buffer             │
   │                 │                          │                           │
   │                 │◀──── AppendEntries ──────┼──── AppendEntries ───────▶│
   │                 │   (prev=1099, 1100..1163)│ fdatasync(log) ║          │
   │                 │ fdatasync(log)           │  (in parallel  ║          │ fdatasync(log)
   │                 │──── ack match=1163 ─────▶│   with sends)  ║          │
   │                 │                          │◀───────── ack match=1163 ─│
   │                 │                          │ own fsync done + 1 ack    │
   │                 │                          │ = majority → commit=1163  │
   │                 │                          │                           │
   │                 │                          │ apply 1100..1163:         │
   │                 │                          │  idx 1137 = our txn       │
   │                 │                          │  compare ModRev==1041? ✔  │
   │                 │                          │  Put → rev 1137           │
   │                 │                          │  → watch hub events       │
   │                 │◀── ProposalResult ───────│                           │
   │                 │   {idx 1137, Succeeded}  │                           │
   │                 │                          │                           │
   │                 │ wait applied ≥ 1137      │                           │
   │                 │ locally (learns commit   │                           │
   │                 │ via next AppendEntries / │                           │
   │                 │ heartbeat, ≤ 1 tick)     │                           │
   │◀── 200 rv=1137 ─│                          │                           │
```

Step-by-step invariants:

1. **Everything non-deterministic happens before Raft.** Admission, defaulting, UID and
   timestamp generation, encryption with a fresh nonce – all in S2's API layer. The Raft
   entry carries final bytes, so every replica's apply is a pure function of the log.
2. **The compare is evaluated at apply time, not at proposal time.** S2 may have checked
   `rv=1041` against its own cache as a fast-fail, but that check is advisory: another
   writer's entry at idx 1120 may have changed the key. The authoritative compare runs
   in the state machine in log order, so two concurrent writers can never both succeed.
   A failed compare takes the `failure` branch, returns the current object, and the API
   layer maps it to `409 Conflict` with the current `rv` attached (saves the client a GET).
3. **Leader fsync runs in parallel with replication** (Raft thesis §10.2.1). The leader
   counts itself towards the majority only once its own fsync is done. With 3 voters
   commit needs leader fsync plus one follower fsync – the slowest disk does not gate
   commit, the second-fastest does.
4. **Group commit amortises fsync.** The batcher closes a batch on 2 ms *or* 256
   proposals *or* 1 MiB, whichever comes first. All 64 entries above share one fsync on
   each node. Under low load a single proposal still pays only one fsync (the batch
   closes immediately when the log writer is idle – no artificial 2 ms delay).
5. **Revisions have gaps.** The no-op entry a new leader commits, membership-change
   entries and `LeaseGrant` entries all consume indexes. Clients must treat `rv` as an
   opaque, monotonically increasing number and never as dense ("next rv is 1138").
6. **Response from the follower waits for local apply.** S2 replies only once its *own*
   `applied ≥ 1137`. This gives read-your-writes for any subsequent read the client
   sends to S2, and S2's watch streams will already have delivered `rev 1137` to
   S2-connected watchers. S2 learns the new commit index from the leader's next
   AppendEntries. The leader sends that message as soon as commit advances, not on the
   250 ms heartbeat tick ([HA defaults](../10-cluster-operations/03-ha-and-failure-modes.md)).
   So the extra cost for follower-originated writes is about one RTT, not one heartbeat
   interval. Without that eager commit broadcast, follower writes would pay up to
   250 ms – worth an explicit test.

Failure cases on this path:

| Failure | Outcome | Client sees |
|---|---|---|
| S1 crashes after appending, before commit | Entry may be committed by the new leader (if S3 or S2 has it) or truncated | `Unavailable{retry_safe: false}` – **ambiguous**. Client must re-read and decide; the API layer attaches the proposal's idempotency key (below) so a blind retry is safe |
| S1 crashes after commit, before replying to S2 | Entry is durable on a majority, will be applied everywhere | Same ambiguity as above, resolved the same way |
| S2 → S1 forward times out | S2 does not know if S1 received it | Same |
| fsync returns `EIO` on S1 | S1 **panics the Raft node and steps down** (never retry fsync – post-`EIO` page-cache state is undefined, the "fsyncgate" lesson). Restart replays from the last good log segment | `Unavailable`, retried transparently by S2 against the new leader |
| Compare fails | Normal path; failure branch | `409 Conflict` with current object |

**Idempotency key** `[proposed]`: each proposal carries `(client_id, request_seq)` and the
state machine keeps a bounded dedup table (last seq per client, TTL 5 min, part of the
snapshot). A retried proposal whose key was already applied returns the original result
instead of applying twice. This closes the ambiguity window for `create` (where a blind
retry would otherwise yield `AlreadyExists` and confuse controllers). etcd/Kubernetes
leave this ambiguity to the client.

### 2. Linearizable read via ReadIndex: `get Deployment web` on S2

```
 Client          S2 (follower)                  S1 (leader)               S3
   │                   │                              │                     │
   │ GET web           │                              │                     │
   │──────────────────▶│                              │                     │
   │                   │ join pending ReadIndex batch │                     │
   │                   │ (≤ 1 ms or 512 reads)        │                     │
   │                   │──── ReadIndexReq ───────────▶│                     │
   │                   │                              │ precondition: an    │
   │                   │                              │ entry of term 7 is  │
   │                   │                              │ committed (the no-op│
   │                   │                              │ after election)     │
   │                   │                              │ read_idx := commit  │
   │                   │                              │        = 1163       │
   │                   │                              │── heartbeat(ctx=r9)▶│
   │                   │◀─ heartbeat(ctx=r9) ─────────│                     │
   │                   │── hb ack(r9) ───────────────▶│◀──── hb ack(r9) ────│
   │                   │                              │ majority confirms   │
   │                   │                              │ S1 still leader     │
   │                   │◀─── ReadIndexResp(1163) ─────│                     │
   │                   │ wait applied ≥ 1163          │                     │
   │                   │ read MVCC at applied         │                     │
   │                   │ (≥ 1163, any later rev fine) │                     │
   │◀── 200 rv=1137 ───│                              │                     │
   │  (mod_rev of obj; │                              │                     │
   │   list rv = read  │                              │                     │
   │   revision)       │                              │                     │
```

Why each step exists:

- **"An entry of the current term is committed"**: a freshly elected leader's commit
  index may lag the true commit point (it can hold committed entries from older terms it
  hasn't yet marked committed). Committing the term's no-op first guarantees its
  `commit` covers every write acknowledged to any client. Reads arriving in this window
  (typically < 1 RTT after election) queue.
- **Heartbeat round with a context tag**: confirms no newer leader exists. Without it, a
  deposed-but-unaware leader (partitioned S1) could serve stale data. The tag `r9` makes
  sure acks are for *this* round, not an older heartbeat.
- **Batching**: one heartbeat round serves every read that joined the batch. At 20k
  reads/s and 1 ms batches this is ~1,000 confirmation rounds/s – cheap (heartbeats are
  small and are sent anyway).
- **Serve at `applied`, not at `read_idx`**: any revision ≥ `read_idx` is linearizable.
  Serving the latest applied state avoids keeping per-read snapshots.

Because the store is the cache, the read is served from the same in-memory structure the
watch hub uses; there is no separate "is the cache fresh?" check (contrast Kubernetes,
which needed KEP-2340 "consistent reads from cache" to get a cheap equivalent – the
apiserver asks etcd for the current revision, then waits until its watch cache has
caught up).

**Read consistency levels exposed by the API:**

| Level | Mechanism | Cost | Use |
|---|---|---|---|
| `Linearizable` (default for GET) | ReadIndex as above | 1 leader RTT (batched) | CLI, controllers' one-off reads |
| `AtLeast(rev)` | wait `applied ≥ rev` locally, no leader contact | 0 RTT if caught up | Read-your-writes after own write; informer resume |
| `Lease` (opt-in) | Leader serves if `now < lease_until` | 0 RTT on leader only | Hot read paths when clocks are trusted |
| `Stale` | local applied state, no wait | 0 | Debug, `rwctl get --stale` |

### 3. Leader failover with leader-local leases

This is the subtle one. Node heartbeats are **not** in the Raft log (see
[registration](../04-node-agent/05-node-registration-and-heartbeat.md)): the leader keeps
a `LeaseTable { node → last_renewal: Instant }` in memory. Node liveness transitions
(`Ready → Unknown`, taints) *are* Raft writes. Parameters: `lease_duration = 20 s`,
renewal every 5 s, election timeout 1–2 s randomised, check-quorum on.

```
time   S1 (leader, term 7)      S2                  S3                Agents (5,000)
─────  ────────────────────     ────────────        ────────────      ──────────────
t=0    LeaseTable live;                                               renew → S1 (via
       node-42 last renewed                                           any server, which
       at t=-19s (it died)                                            forwards)
t=0    ✖ S1 crashes
t=0..                           no heartbeat        no heartbeat      renewals fail /
1.4s                                                                  time out (retry
                                                                      with backoff, try
                                                                      other servers)
t=1.4s                          election timeout
                                PreVote(term 8) ───▶ grants
                                RequestVote(8) ───▶ votes
t=1.5s                          LEADER term 8
                                append no-op idx 1200
t=1.52s                         commit 1200 (own + S3 fsync)
                                ── lease takeover ──
                                for every Node in store:
                                  last_renewal := now (t=1.52s)
                                  i.e. grace = full lease_duration
                                node lifecycle task starts here: it
                                is pinned to the Raft leader (the
                                lease table lives there), fenced by
                                the Raft term, not by a separate
                                controller lease
t=1.6s                                                                agents discover
..4s                                                                  leader (redirect);
                                                                      jittered renewals
                                                                      → LeaseTable fills
t=21.52s                        node-42: no renewal since takeover
                                → propose Txn{ if leader_epoch==8 ∧
                                   node-42.status.rev==X,
                                   set Unknown + unreachable taint }
t=21.55s                        committed; pods on node-42 enter
                                eviction/replacement per tolerations
```

Invariants and reasoning:

1. **A new leader has no lease history, so it assumes everyone is alive.** Setting every
   node's `last_renewal := takeover_time` means no node can be declared dead until a full
   `lease_duration` has passed under the new leader. Since a live agent renews every 5 s
   and failover takes ~1.5 s, every live agent gets ≥ 3 attempts in the grace window.
   This is what makes non-durable leases safe: **false positives are impossible by
   construction; the cost is slower true-positive detection.**
2. **Worst-case detection delay** for a node that died just before failover:
   `time_since_last_renewal_before_crash + election_time + lease_duration`
   ≈ 19 s + 1.5 s + 20 s ≈ **40.5 s** (vs 20 s steady state) – roughly 2× lease duration.
   Accepted trade-off, tracked as `PP-R3`. Mitigation `[open]`: the leader replicates a
   compact "suspect list" (nodes with no renewal for > ½ lease) through Raft every 10 s;
   a new leader starts those nodes' clocks from the replicated hint instead of from
   takeover. Costs one small write per 10 s cluster-wide – still ~4 orders of magnitude
   fewer writes than durable heartbeats.
3. **A partitioned old leader cannot act on its stale lease table.** Variant: S1 doesn't
   crash but is partitioned from S2/S3 (agents can still reach S1 in some cases):

   ```
   S1 (partitioned)                      S2 (new leader, term 8)
   still believes term 7 for ≤ election
   timeout; check-quorum fails → steps
   down to follower at t≈1–2 s
   during that window its lease table
   may show nodes "expired" (renewals
   now going to S2) → proposes
   Txn{leader_epoch==7, set Unknown}
   └─ can't reach a majority → never
      commits; after step-down the
      proposal is dropped and the
      caller gets NotLeader
   ```
   Two independent fences: (a) the write needs a quorum S1 doesn't have; (b) even if a
   proposal were somehow delayed and committed later, the `leader_epoch == 7` compare
   fails once the term-8 leader has committed its first entry, which records `leader_epoch =
   8` in the store (the same lease-token precondition idea as PP-41, keyed on the Raft
   term instead of a controller lease).
4. **Agents must not self-fence during failover.** Agent self-fencing triggers after
   `lease_duration + safety_margin` (20 s + 5 s) without a successful renewal. Failover
   (~1.5 s) plus herd catch-up (~3 s) is far inside that. The server's reschedule wait
   for singleton/exclusive-volume pods is `lease_duration + safety_margin +
   drift_allowance`, measured from the takeover, so fencing remains ordered: the agent
   always stops its pods before the server can start replacements.
5. **Renewal thundering herd.** All 5,000 agents retry within seconds of the election.
   Renewals are in-memory map updates on the leader (no Raft, no fsync), so the herd is
   ~5,000 tiny RPCs – but agents still use full jitter (`retry_in = rand(0, min(cap,
   base·2^n))`) and the leader admits renewals in their own fairness level so they can
   never be starved by, or starve, normal API traffic
   ([API fairness](09-api-fairness.md)).
6. **Planned leadership transfer** (`rwctl server step-down`, rolling upgrades): the old
   leader sends `TimeoutNow` to a caught-up follower **and hands over its lease table**
   in the transfer message. The new leader uses the transferred table instead of the
   full-grace takeover – planned failovers don't extend detection at all.
7. **Leader lease reads (optional) and failover.** If `Lease` reads are enabled, the old
   leader's read lease must expire before any new leader can be elected:
   `read_lease = election_timeout_min − max_clock_drift` (e.g. 1 s − 50 ms). Raft's
   election timeout guarantees no other node becomes leader sooner, *assuming bounded
   drift between monotonic clocks*. This is the only place the design depends on clock
   rate, which is why it's opt-in.

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
