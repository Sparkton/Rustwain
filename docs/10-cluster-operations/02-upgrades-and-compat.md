# Upgrades and Compatibility

**K8s analogue:** version skew policy (kubelet up to 3 minor versions older than
apiserver since 1.28; apiserver HA skew 1 minor), kubeadm upgrade, API deprecation policy
(GA APIs not removed within a major version; beta 9 months/3 releases), storage version
migration, feature gates (alpha/beta/GA), release cadence (3/year, 14-month support).

## Pain points in Kubernetes upgrades
- Must upgrade one minor version at a time (1.28 → 1.29 → 1.30…); with 3 releases a year
  and ~14 months of support, clusters fall behind and face multi-step upgrade marathons.
- API removals (e.g. `extensions/v1beta1` Ingress, PSP) break manifests and Helm charts.
- Node upgrades drain and restart every pod (kubelet/runtime upgrades usually require it in
  managed offerings).
- Add-ons (CNI, CSI, CoreDNS, ingress) each have their own compatibility matrix.

## Rustwain versioning
- SemVer for the product: `MAJOR.MINOR.PATCH`.
- API versions per group (`v1`, `v1beta1`) independent of product version.
- Compatibility promises:
  - **API**: `v1` APIs never removed within a major product version. Beta APIs supported
    for at least 2 minor releases after their replacement ships, with conversion.
  - **Skew**: agents may be up to **2 minor versions** older than servers; servers within a
    cluster may differ by 1 minor during rolling upgrades.
  - **Supervisor protocol**: agent N supports supervisors from N−2 (supervisors are long-lived:
    they live as long as the pod).
  - **Raft log/snapshot format**: versioned; server N reads N−1 formats; format bumps are
    gated by a cluster-wide **feature level** that is raised only after all servers run the
    new version (like CockroachDB's cluster version / finalisation).
  - **BPF map schemas**: versioned names with migration (see dataplane doc).
  - **Agent checkpoint DB**: versioned schema with migrations, downgrade within one minor
    supported (keep old columns until finalisation).

## Upgrade flow

### Servers (rolling, one at a time)
1. `rwctl upgrade plan` – checks: skew, deprecated API usage in stored objects and in
   recent requests (from audit/metrics), feature level, quorum health, backups recent.
2. Download + verify signed binary on each server.
3. For each server: transfer leadership away if leader → stop → swap binary → start →
   wait for caught up and healthy.
4. After all servers upgraded: `rwctl upgrade finalize` raises the cluster feature level
   (enables new formats/behaviours; irreversible).

### Agents (in place, no workload disruption)
Because supervisors keep pods running and BPF maps are pinned (see
[node agent](../04-node-agent/01-node-agent-architecture.md)):
1. Agent downloads new binary (from servers – servers can serve the release artifact to
   agents → air-gap friendly), verifies signature.
2. `execve` itself into the new binary (or systemd restart).
3. New agent recovers state, re-attaches supervisors, replaces BPF programs atomically.
4. **No drain, no pod restarts.** This is a major operational improvement.
- Rate-limited across the cluster by an `AgentUpgrade` rollout object (maxUnavailable,
  pause on failure, canary nodes first).

### Kernel / OS upgrades
Not Rustwain's job, but we provide the drain orchestration (`Drain` objects and
`MaintenancePolicy`) that OS update tools (e.g. kured-like reboot coordinators) can use.
Built-in reboot coordinator `[later]`: watches `/var/run/reboot-required` (Debian/Ubuntu) or
`needs-restarting` equivalent and schedules drains/reboots within policy.

### Supervisors
Never upgraded in place; replaced naturally as pods restart. Long-lived pods may run old
supervisors for months – supported by protocol N−2 and a soft condition
`SupervisorOutdated` + optional "recycle on next rollout" policy.

## Feature gates
- Features progress `alpha` (off by default, may change/disappear) → `beta` (on by default)
  → `GA` (gate removed).
- Fewer gates than Kubernetes; a gate must have an owner, a doc link and a removal target.

## Deprecation tooling
- API server records usage of deprecated API versions/fields per client (user agent +
  identity) → `rwctl upgrade plan` reports who still uses them.
- Warnings returned on every deprecated request (`Warning` header).
- `rwctl convert` rewrites manifests to new versions.

## Downgrades
- Patch downgrades: always.
- Minor downgrade: supported before `finalize` (formats unchanged until then).

## Open questions

- Skipping minor versions for agents (N−2 support allows upgrading servers twice, then
  agents once).
- Agent self-exec upgrade vs systemd-managed restart (self-exec avoids systemd dependency).

## Build checklist

- [ ] Version/skew negotiation in every protocol handshake (agent↔server, supervisor↔agent,
      server↔server).
- [ ] Cluster feature level + finalize.
- [ ] Rolling server upgrade with leadership transfer.
- [ ] Agent in-place upgrade without disruption (integration test with live traffic).
- [ ] Deprecated usage tracking + `rwctl upgrade plan`.
