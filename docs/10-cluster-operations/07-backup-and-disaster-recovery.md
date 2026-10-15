# Backup and Disaster Recovery

**K8s analogue:** `etcdctl snapshot save/restore`, etcd backup operators, Velero (API
object + volume backups to object storage, restic/kopia file-level backup, CSI
snapshots), Kasten K10.

## What needs backing up
| Data | Where | Method |
|---|---|---|
| Cluster state (all API objects) | Store | Store snapshots |
| Cluster PKI (CA keys) | Store (encrypted) / offline | Included in snapshot (encrypted with KEK) + root CA offline |
| KEK (encryption key) | Server key file / KMS | **Separately** – a backup without the KEK is unreadable for secrets |
| Persistent volume data | Storage backends | Volume snapshots + export |
| Agent local state | Nodes | Not backed up – reconstructible |

## Store backups
- `rwctl store snapshot save s3://bucket/prod/2026-10-15.snap` (or local file) – streams a
  consistent snapshot at a revision from a server (follower preferred to spare the leader).
- **Scheduled backups**: `BackupSchedule` object (cron, destination, retention); executed
  by a controller on a follower; uploads to S3-compatible storage (Rust `object_store` or
  `aws-sdk-s3` crate; plugin for others).
- Snapshot format: versioned, checksummed, compressed (zstd), includes cluster ID,
  revision, feature level.
- Encryption: snapshot contains secrets as ciphertext (already DEK-encrypted); the file
  is additionally encrypted with a backup key `[proposed]`.

### Restore
- `rustwain server restore --snapshot <file> --data-dir ...` → creates a **new
  single-member cluster** from the snapshot (same cluster ID, same CA), then add servers.
- Revision continuity: restored cluster continues from the snapshot revision + a large
  offset `[proposed]` (so clients with cached revisions from after the snapshot don't get
  confused – they get `TooOld` and relist). Kubernetes/etcd restores reset revisions,
  which can confuse controllers' caches (etcd has `--bump-revision` and `--mark-compacted`
  for exactly this reason).
- After restore, agents reconnect; their actual state may differ from restored desired
  state (pods that exist but aren't in the snapshot) → reconciled: unknown pods are
  terminated after a grace period; `rwctl` warns loudly before allowing this (`--adopt`
  option to import unknown pods as orphaned objects instead).

## Application-level backup `[later]`
Velero-like: back up a namespace's objects + its volumes, restore into another namespace
or cluster.
- Objects: filtered export from the API (exclude status/system fields; include
  dependencies like Secrets/Configs/Claims).
- Volumes: driver snapshots, optionally **exported** to object storage (incremental: see
  [local storage](../06-storage/03-local-storage.md)) for cross-cluster restore.
- Hooks: pre/post-backup exec hooks for app consistency (e.g. `pg_start_backup`), freeze
  via `cgroup.freeze` for crash consistency `[proposed]`.

## DR scenarios
| Scenario | Procedure | RPO | RTO |
|---|---|---|---|
| Single server loss | Automatic (Raft) | 0 | seconds |
| Quorum loss, data on a survivor | `server recover --force-new-cluster` | ~0 (uncommitted lost) | minutes |
| All servers lost | Restore latest snapshot to new servers | backup interval | minutes–hour |
| Region loss | Restore snapshot + volume exports in another region | backup interval | hours |
| Accidental deletion of a namespace | App-level restore of that namespace | backup interval | minutes |

## Open questions
- Point-in-time recovery via shipping the Raft log continuously (WAL archiving)? Nice:
  RPO ≈ seconds. `[later]`.

## Build checklist
- [ ] Snapshot save/restore commands; object storage upload.
- [ ] BackupSchedule controller with retention.
- [ ] Restore with revision bump and agent reconciliation safeguards.
- [ ] DR drill test in CI (restore snapshot into fresh cluster, workloads converge).
