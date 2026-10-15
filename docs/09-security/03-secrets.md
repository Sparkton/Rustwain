# Secrets

**K8s analogue:** Secret objects, EncryptionConfiguration (aescbc, aesgcm, secretbox,
KMS v1/v2 providers), kubelet secret delivery (tmpfs), Secrets Store CSI driver,
External Secrets Operator, Sealed Secrets, SOPS.

## Threats

| Threat | Mitigation |
|---|---|
| Store/backup theft | Encryption at rest with envelope encryption |
| Over-broad API read access | RBAC; secrets not readable by default roles beyond namespace admins; `list` of secrets restricted (list returns all values in Kubernetes!) |
| Node compromise | Node authorizer: a node only gets secrets for its own pods |
| Pod compromise | Only that pod's secrets on its tmpfs; no default SA token |
| Leak via env/logs | Lint against env; log redaction is not possible generically |
| Secrets in Git | Not our problem directly, but support sealed/encrypted manifests via plugins |
| Memory scraping on node | `memfd_secret` / mlock for agent-held secrets `[later]` |

## Encryption at rest

Envelope encryption:
- Each secret value encrypted with a **DEK** (AES-256-GCM, random 96-bit nonce per
  encryption). Kubernetes KMS v2 uses a DEK per key-generation
  epoch (reused across many writes, with a seed + nonce scheme to stay within AES-GCM
  limits). We follow KMS v2's approach: DEK rotated periodically (or after N encryptions),
  wrapped by the **KEK**.
- KEK providers:
  - `local`: key file on each server (simple; protect file + disk encryption).
  - `tpm` `[later]`: sealed to server TPM.
  - `kms`: external KMS via plugin (gRPC, compatible with Kubernetes KMS v2 plugin protocol
    `[proposed]` → reuse existing AWS/GCP/Azure/Vault KMS plugins).
- Encrypted in the API layer before Raft proposal → Raft log, snapshots and backups only
  contain ciphertext.
- Key rotation: new KEK → background re-wrap of DEKs (cheap: only DEK headers change) →
  old KEK retired. `rwctl secrets rotate-kek`.
- Which kinds are encrypted: Secrets always; configurable for others (e.g. custom types
  holding credentials).

## Access semantics changes
- `list` and `watch` on secrets: Kubernetes returns full values, so "list" permission =
  read all secrets. Rustwain `[proposed]`: `list`/`watch` return **metadata only** unless
  the caller also has `get-values` (a separate verb). Controllers that need values watch
  with explicit permission.
- Read access audited at metadata level by default (who read which secret).

## Delivery to nodes
- Via node bundle stream, only for pods on that node.
- Written to per-pod **tmpfs** (never disk; swap: tmpfs pages can be swapped – on nodes
  with swap, use `memfd_secret`-backed delivery or `noswap` tmpfs mount option (6.4+) for
  secret volumes `[proposed]`).
- Agent forgets secret values after materialisation? It must re-materialise after pod
  restarts without API access (disconnected operation) → keep in agent memory (encrypted
  with a node-local key) rather than disk. Trade-off documented.

## External secrets `[later]`
Provider plugins (Vault, AWS SM, GCP SM, Azure KV): sync-to-Secret mode or
mount-time-only mode (never stored in cluster). See
[config delivery](../08-workloads/05-config-and-secret-delivery.md).

## Open questions

- Metadata-only list semantics – breaks compatibility with tools expecting values; worth
  it for security.
- noswap tmpfs requires 6.4+; fallback when swap is on and kernel older: warn.

## Build checklist

- [ ] Envelope encryption with DEK epochs; local KEK provider.
- [ ] KMS v2-compatible plugin client.
- [ ] `get-values` verb semantics for list/watch.
- [ ] Rotation commands and background re-wrap.
- [ ] noswap tmpfs for secret volumes when available.
