# High Availability and Failure Modes

**K8s analogue:** HA control plane (stacked/external etcd, apiserver behind LB, leader
election for scheduler/controller-manager), node failure handling, etcd quorum loss
recovery, zone failures, "static stability".

## Principle: static stability
When the control plane is unavailable, the **data plane keeps working**: running pods keep
running and restarting per policy, services keep load balancing to the last known
endpoints, DNS keeps answering from the last known state, network policy stays enforced.
Nothing is torn down for lack of contact. (Kubernetes mostly behaves this way, by
accident of design; we make it an explicit, tested requirement.)

## Failure matrix

| Failure | Detection | Impact | Recovery |
|---|---|---|---|
| One server down (of 3) | Raft heartbeats | None for clients (other servers serve); controllers fail over (lease expiry ~15 s) | Restart or replace server; it catches up via log/snapshot |
| Raft leader down | Raft election timeout (~1–2 s) | Writes pause ~1–3 s | New leader; leader-local node leases get grace period |
| Quorum lost (2 of 3 down) | Raft | **No writes**; reads from caches possibly stale (consistent reads fail); data plane unaffected | Bring servers back; or **disaster recovery**: `rustwain server recover --force-new-cluster` from a surviving member's data (accepts possible loss of last uncommitted writes) |
| All servers down | Agents can't connect | Data plane static; no changes possible | Restart servers; agents reconnect and reconcile |
| Server disk full | Store write failures | Writes fail | Alarms early (store size quota); compaction/snapshot; disk expansion |
| Slow disk on a server (fsync latency) | Raft metrics | Elevated write latency; possibly leader churn | Leader transfer away from slow member; alarms |
| Agent down (process) | Supervisor sockets idle; lease expiry | Pods keep running; no status updates; crashed containers restarted by their supervisor per policy after 5 s of disconnection (probe-driven restarts wait for the agent) | systemd restarts agent; recovers state |
| Node down (power/kernel panic) | Lease expiry (20 s) | Pods lost | Node lifecycle controller → replacements after toleration windows; exclusive volumes wait for fencing |
| Node partitioned from servers only | Lease expiry | Pods keep serving local/peer traffic; replacements may start elsewhere (stateless) | Self-fencing for singleton/exclusive-volume pods; on heal, agent deletes pods that were replaced |
| Zone outage | Many leases expire in one zone | Capacity loss | Zone-aware eviction rate limiting; topology spread places replacements in other zones |
| Control-plane network partition (minority side) | Raft | Minority servers reject writes; their API serves stale reads only if `consistency=any` | Heal |
| Bad config pushed (policy denies everything) | Users | Writes rejected | Built-in exclusion of `rustwain-system` from user policies; break-glass admin bypass; dry-run before enforce |
| Bad rollout of a workload | Readiness/progress deadline | Partial outage of that app | Rollout pause/auto-rollback |
| Webhook down | Timeouts | Writes to matching resources fail/ignored | Circuit breaker; excluded system namespaces |
| Certificate expiry | Proactive rotation | (Shouldn't happen) | Rotation scheduler; alarms at 50% lifetime remaining if renewal failing |
| Clock skew | Heartbeat timestamp comparison (informational) | Certificate validity issues | NTP; correctness does not depend on clock sync |
| Store corruption (bug) | Checksums on snapshots/log entries | Member can't start | Wipe member, rejoin from peers; or restore from backup |

## Supervisor autonomy during agent outage `[proposed]`
If the agent is down for minutes (crash loop due to a bug), crashed containers aren't
restarted because restart policy lives in the agent. Option: give the supervisor the
pod's restart policy and backoff, so it restarts containers itself and reports later.
Pro: better static stability. Con: a bigger supervisor that is never upgraded in place.
Decision: yes, for the restart policy, exit-code rules and backoff only, using one pure
function shared by agent and supervisor; probes stay in the agent. The handover protocol
is in [pod lifecycle](../04-node-agent/02-pod-lifecycle.md#who-restarts-containers-while-the-agent-is-down-proposed).

## Control-plane sizing and placement
- Servers on separate failure domains (zones/racks).
- Odd number of voters; learners for read scaling in other zones.
- Raft heartbeat/election timeouts tuned to RTT between zones (default heartbeat 250 ms,
  election 1–2 s for LAN; scale for WAN).

## Disaster recovery
See [backup and DR](07-backup-and-disaster-recovery.md). Quorum loss recovery procedure:
1. Choose the surviving member with the highest committed index (`rustwain server
   inspect --data-dir`).
2. `rustwain server recover --force-new-cluster` → new single-member cluster with that data
   (new cluster membership, same cluster ID and CA).
3. Join new servers.
4. Agents reconnect automatically (same CA, same endpoints/LB).

## Testing HA (non-negotiable)
- Chaos suite in CI VMs: kill -9 servers/agents at random, partition networks
  (`tc netem`, iptables-free BPF drops), fill disks, slow disks (`dm-delay`), skew clocks;
  assert invariants (no lost acknowledged writes, no double-running singleton pods,
  convergence after healing within bound).
- Deterministic simulation for the store and controllers (see state store doc).

## Open questions

- Should agents be able to fetch state from *peer agents* when servers are down (e.g. new
  endpoints)? No – static stability means no changes, which is the simpler, safer model.

## Build checklist

- [ ] Static stability integration tests (servers down for 10 minutes; traffic keeps
      flowing; crashed containers restarted if supervisor autonomy adopted).
- [ ] `server recover --force-new-cluster` and `server inspect`.
- [ ] Chaos test suite with invariant checkers.
- [ ] Zone-aware eviction rate limiting.
