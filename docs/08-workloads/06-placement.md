# Placement: Affinity, Taints, Topology Spread

**K8s analogue:** nodeSelector, node affinity (required/preferred), pod affinity and
anti-affinity, taints and tolerations (NoSchedule, PreferNoSchedule, NoExecute with
tolerationSeconds), topology spread constraints, node labels (well-known
`topology.kubernetes.io/zone`, `kubernetes.io/hostname`, `kubernetes.io/arch`).

## Node attributes vs labels

Kubernetes uses string labels for everything (user-defined and system-detected via
Node Feature Discovery, e.g. `feature.node.kubernetes.io/cpu-cpuid.AVX512F=true`).

Rustwain separates:
- **Labels**: user-defined, string, for grouping (`pool=batch`, `team=a`).
- **Attributes**: system-detected, **typed** (bool, int, version, string), namespaced
  (`kernel.version=6.8.0`, `cpu.avx512=true`, `mem.total=512Gi`, `gpu.count=8`,
  `topology.zone=eu-west-1a`, `isolation.microvm=true`, `arch=arm64`,
  `page_size=4096`). Users can't spoof attributes (agent-reported; admins can override).

Selectors work on both, with typed comparisons for attributes:
```yaml
placement:
  require: 'attr["arch"] == "arm64" && attr["kernel.version"] >= v"6.6" && label["pool"] == "web"'
  prefer:
    - weight: 50
      expr: 'attr["cpu.avx512"]'
```
(Expressions in the same CEL dialect as policies/devices; simple map form also accepted
for common cases: `nodeSelector: {pool: web}`.)

## Taints and tolerations
Kept as in Kubernetes (well understood):
- Taint `{key, value, effect}` on nodes; effects `NoSchedule`, `PreferNoSchedule`,
  `NoExecute` (evicts running pods without toleration, after `tolerationSeconds`).
- System taints: `unreachable`, `not-ready`, `memory-pressure`, `disk-pressure`,
  `pid-pressure`, `network-unavailable`, `unschedulable` (cordon), `out-of-service`.
- Default tolerations per workload type (DaemonSets tolerate most system taints).
- Tolerations with operator `Exists` / `Equal`; plus typed comparison `[proposed]`
  (`Lt`/`Gt` numeric) – Kubernetes added numeric comparison operators for tolerations
  only recently (alpha).

## Pod affinity / anti-affinity
"Place near pods matching X" / "away from pods matching X", within a **topology domain**
(node, zone, rack):
```yaml
placement:
  antiAffinity:
    - selector: { app: db }
      topology: node            # required: no two db pods on the same node
  affinity:
    - selector: { app: cache }
      topology: zone
      preferred: { weight: 80 }
```

### Performance
Kubernetes' inter-pod affinity used to be the slowest scheduler feature (evaluating
existing pods across all nodes). Rustwain's scheduler keeps **per-domain counters of
label-matching pods** (indexed by the selectors in use) updated incrementally → each
affinity check is O(1) per node.

Symmetry: anti-affinity must be checked both ways (existing pods' anti-affinity against
the incoming pod). Indexed the same way.

## Topology spread constraints
"Spread replicas evenly across zones with max skew 1":
```yaml
placement:
  spread:
    - topology: zone
      maxSkew: 1
      whenUnsatisfiable: DoNotSchedule    # or ScheduleAnyway
      minDomains: 3
      selector: owner                      # default: pods of the same owner/revision
      matchRevision: true                  # [proposed] count only the current revision
                                           # (k8s 1.27 matchLabelKeys: [pod-template-hash])
```
Known Kubernetes pitfalls addressed by defaults:
- Counting old-revision pods during rollouts skews placement → `matchRevision: true`
  default.
- Nodes with taints the pod doesn't tolerate counted as domains → `nodeTaintsPolicy:
  Honor` default.
- Scale-down doesn't rebalance → rebalancer (see scheduler doc).

## Default spreading
Without explicit constraints, the scheduler softly spreads replicas of the same owner
across nodes and zones (Kubernetes has default `PodTopologySpread` with
`ScheduleAnyway`). Keep.

## Node pools
A `NodePool` object `[proposed]` groups nodes with shared labels/taints/config (and
autoscaling provider settings) – a first-class version of the informal "node group"
concept – so `placement: { pool: gpu }` is simple and validated (unknown pool → error, not
pending forever).

## Open questions

- How much to simplify the placement API vs Kubernetes' nested
  `affinity.nodeAffinity.requiredDuringSchedulingIgnoredDuringExecution.nodeSelectorTerms`
  structure (very verbose). Proposal above is much flatter; import adapter translates.
- `RequiredDuringExecution` (evict if constraints become violated) – never implemented in
  Kubernetes; skip (rebalancer can handle soft cases).

## Build checklist

- [ ] Attribute model (typed) + agent publication + admin overrides.
- [ ] Placement API types and expression compilation.
- [ ] Scheduler plugins with domain counters for affinity and spread.
- [ ] NodePool kind.
