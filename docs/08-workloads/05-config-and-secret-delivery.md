# Config and Secret Delivery to Workloads

**K8s analogue:** ConfigMap, Secret (as env vars and volumes), downward API, projected
volumes (serviceAccountToken, clusterTrustBundle), `subPath`, immutable ConfigMaps,
kubelet secret/configmap managers (watch/cache/TTL), Secrets Store CSI driver,
Reloader-style restart-on-change tools.

## Objects
- `Config` (≈ ConfigMap): string and binary keys, ≤ 1 MiB total, optional `immutable`.
- `Secret`: same shape, encrypted at rest, stricter RBAC defaults, typed variants
  (`tls`, `registry-auth`, `basic-auth`, `opaque`).

## Delivery mechanisms

### Environment variables
- `env: [{name, config: {name, key}}]` or `secret:`; `envFrom` for all keys.
- Values fixed at container start (process env can't change) – a change requires restart.
- Secrets in env are visible in `/proc/<pid>/environ` to same-UID processes and often
  leak into crash dumps and logs. Lint warns for secrets in env; prefer files.

### Files (volumes)
- Materialised by the agent into a **per-pod tmpfs** (secrets never touch disk).
- **Atomic updates**: write new version into a timestamped dir, then atomically swap a
  `..data` symlink (Kubernetes' `AtomicWriter` pattern – keep it; apps watching with
  inotify see a consistent set of files).
- `subPath` mounts of a single file **don't receive updates** in Kubernetes (bind mount
  pins the inode) – a long-standing confusion. Rustwain: `file` mounts that point at the
  atomic symlink path rather than bind-mounting the inode `[proposed]`, or clearly
  marked as `static` in status.
- Ownership/permissions: `defaultMode`, per-key `mode`, owned by pod's mapped GID.

### Downward API
Pod metadata (name, namespace, uid, labels, annotations, node name, IPs, service
account) and resource requests/limits exposed as env or files. Labels/annotations files
update live.

### Projected volumes
Combine several sources (config, secret, downward, service account token, trust bundle)
into one directory.

## Update propagation

Kubernetes: kubelet watches (or polls with TTL) each referenced ConfigMap/Secret; update
latency = kubelet sync period + cache TTL (up to ~1–2 minutes), and each pod/secret pair
can be its own watch (at 100k pods with unique secrets, this is a lot of watches; immutable
secrets were introduced partly to reduce this load).

Rustwain:
- The **node bundle stream** (see [node agent](../04-node-agent/01-node-agent-architecture.md))
  delivers referenced configs/secrets for the node's pods on one watch, server-filtered by
  the node authorizer.
- Update to files within ~1 s of the API write.
- Immutable objects are delivered once, never watched.

### Reacting to changes `[proposed]`
Apps differ: some reload on file change, some need SIGHUP, most need a restart.
```yaml
volumes:
  - name: conf
    config: { name: web-conf }
    onChange: Signal(SIGHUP)     # None (default) | Signal(sig) | RestartContainer | Rollout
```
- `RestartContainer`: agent restarts the container in place (respecting a jitter to avoid
  restarting all replicas at once – or better:)
- `Rollout`: the workload controller includes a hash of referenced configs in the template
  hash → a change triggers a normal rolling update (safe, respects budgets, can be
  rolled back). This is what people do with Helm checksum annotations or Reloader.
  Native support is a meaningful quality-of-life improvement.

## External secret stores
- Pull secrets from Vault/AWS Secrets Manager/GCP Secret Manager:
  - Option A: sync controller (External Secrets Operator model) – external → Secret object.
  - Option B: node-side fetch at mount time (Secrets Store CSI model) – never stored in the
    cluster store.
- Rustwain `[later]`: provider plugin interface supporting both modes; B is safer for high
  sensitivity secrets.

## Size and scale limits
- 1 MiB per object; total config/secret bytes per namespace quota.
- Binary data fine (no base64 on our binary wire; base64 only in YAML/JSON rendering –
  Kubernetes' base64 in Secrets causes endless "is base64 encryption?" confusion; our CLI
  shows `stringData` by default when values are UTF-8).

## Open questions

- Should `Rollout` on config change be the default for Deployments? Safer behaviour by
  default but surprising if users expect live reload. Lean: default `None`, lint
  suggests.

## Build checklist

- [ ] Config/Secret kinds; immutability.
- [ ] Agent materialiser with atomic symlink swap, tmpfs per pod.
- [ ] Node bundle stream integration.
- [ ] `onChange` behaviours, including config hash in template hash.
- [ ] Downward API including live label/annotation files.
