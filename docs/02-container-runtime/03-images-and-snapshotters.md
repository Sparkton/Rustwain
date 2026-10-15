# Images, Registries and Snapshotters

**K8s analogue:** containerd image service + content store + snapshotter, kubelet image
manager and image GC, image credential providers, `imagePullPolicy`, image volumes.

## OCI image format refresher

```
Reference: registry.example.com/team/app:1.4@sha256:abcd...
                 │                 │      │      └─ digest (immutable)
                 │                 │      └─ tag (mutable pointer)
                 │                 └─ repository
                 └─ registry host

Image index (application/vnd.oci.image.index.v1+json)        ← multi-arch "manifest list"
 ├─ manifest for linux/amd64
 ├─ manifest for linux/arm64/v8
 └─ (attestations / signatures as referrers, artifactType)

Manifest (application/vnd.oci.image.manifest.v1+json)
 ├─ config  → blob: image config JSON (entrypoint, env, user, workdir, rootfs.diff_ids, history)
 └─ layers  → [blob: tar+gzip | tar+zstd | tar, ...]
```

- Everything is **content-addressed** by `sha256` (or `sha512`) digest.
- Layers are tarballs; the config lists `diff_ids` (digests of *uncompressed* tars) – the
  layer identity used for snapshot chaining (`chainID = sha256(parentChainID + " " + diffID)`).
- Docker v2 schema 2 manifests are equivalent and must also be supported.

## Distribution protocol (registry API)

- `GET /v2/<name>/manifests/<ref>` with `Accept` headers for index/manifest media types.
- `GET /v2/<name>/blobs/<digest>` – often a 307 redirect to object storage (S3/GCS
  signed URL). Must follow redirects without leaking the registry auth header to the
  redirect host.
- **Auth:** `WWW-Authenticate: Bearer realm=...,service=...,scope=repository:x:pull` →
  fetch token from realm with Basic creds (or anonymous) → retry with
  `Authorization: Bearer`. Docker Hub, GHCR, ECR (via credential helper), GCR/GAR, ACR all
  follow variants of this.
- Range requests on blobs: supported by most registries → resumable downloads and
  parallel chunked downloads.
- Referrers API (OCI 1.1): find signatures/SBOMs attached to an image.

Rust crates: `oci-client` (formerly `oci-distribution`, from the Krustlet/Wasm
community) covers auth and pulls; `oci-spec` (from youki) for types. We'll likely use
`oci-spec` types and our own pull engine on `hyper`/`reqwest` for control over
parallelism, retries and caching.

## The image store

```
/var/lib/rustwain/images/
├─ content/blobs/sha256/<digest>      ← raw blobs (compressed layers, manifests, configs)
├─ content/ingest/<ref>/              ← in-progress downloads (resumable)
├─ snapshots/overlay/<id>/fs          ← unpacked layer dirs
├─ snapshots/overlay/<id>/work
├─ l/<shortid> -> ../snapshots/...    ← short symlinks for overlay option length
└─ meta.redb                          ← metadata DB: refs, leases, snapshot chain, usage, last-used
```

Metadata DB: `redb` (pure Rust embedded KV, ACID) – same engine family as the agent
checkpoint, see [crate choices](../13-implementation/02-rust-ecosystem.md).

### Leases and GC roots
A blob or snapshot is kept alive while referenced by:
- a pod on this node (running or pending),
- an in-progress pull (lease with TTL),
- a pinned image (DaemonSet/system images, configured pins).
Everything else is eligible for GC by LRU when disk thresholds are crossed. See
[image and disk GC](../04-node-agent/07-image-and-disk-gc.md).

## Pull pipeline

```
resolve ref → (tag → digest via HEAD/GET manifest, honouring pull policy)
   → select platform manifest from index (os/arch/variant/os.version, features)
   → verify signature policy (optional, see supply chain)
   → fetch config blob
   → for each layer in parallel (bounded):
        if snapshot for chainID exists: skip
        else: stream download → hash while downloading → decompress (gzip/zstd) streaming
              → untar into snapshot dir (sandboxed unpacker) → verify diff_id
   → commit snapshots in order (chain)
   → record image ref → manifest digest
```

Key efficiency points:
- **Stream, don't stage**: download → decompress → untar in one pipeline per layer; never
  write the compressed blob and then re-read it (keep the compressed blob only if we
  want to serve it to peers – `[open]`, see P2P below).
- **Parallel layers, ordered commit**: unpack layers concurrently into separate dirs (each
  layer unpack is independent in overlay mode!) – only the chain metadata is ordered.
  containerd historically unpacked sequentially.
- **zstd**: 2–3× faster decompression than gzip; prefer `zstd` layers when the registry
  offers them (zstd:chunked). Use `zlib-rs`/`flate2` with the fastest backend for gzip.
- **Dedup in-flight pulls**: two pods needing the same image share one pull (singleflight).
- **Bounded concurrency**: per-registry and global limits so a DaemonSet rollout doesn't
  DDoS the registry; obey `429` with backoff.
- **Pull progress** surfaced in pod status ("pulling 3/7 layers, 412 MiB/1.2 GiB") –
  Kubernetes only shows "Pulling image" with no progress.

### Untrusted tar handling (security)
The unpacker parses attacker-controlled data. It runs in a **separate sandboxed process**
(seccomp: only file ops; Landlock: only the target snapshot dir; no network; userns) and
enforces:
- No absolute paths, no `..` traversal; resolve within target with `openat2(RESOLVE_BENEATH)`.
- Hardlinks only to entries inside the same layer root.
- Symlinks are created as-is but **never followed** during unpack.
- Device nodes: skipped (or only allowed in privileged images policy), never created
  with `mknod` on the host.
- setuid/setgid bits preserved (needed for e.g. `sudo` in images) but meaningless under
  NNP + userns; policy may strip them.
- xattrs: allow `user.*`, `trusted.overlay.*` handling for whiteouts; drop `security.*`
  except `security.capability` (file caps), which is rewritten for userns (v3 file caps
  with rootid).
- Limits: max entries, max path length, max total size (configurable), to stop zip bombs.
- Whiteouts: `.wh.<name>` → overlay whiteout (char 0/0 device – needs `CAP_MKNOD` in
  userns ≥ 5.8? or use `user.overlay.whiteout` xattr format with `userxattr` overlay
  option for unprivileged). `.wh..wh..opq` → opaque dir xattr.

## Pull policy semantics

Kubernetes' `imagePullPolicy` defaults depend on the tag (`:latest` → Always, else
IfNotPresent) – surprising. Rustwain:

- References are **resolved to a digest at admission** (by the API server or controller,
  `[proposed]`) and stored in pod status. All replicas of a Deployment then run the
  *same bytes* even if the tag moves mid-rollout. (Kubernetes can run mixed versions if a
  tag is re-pushed during a rollout.)
- Policy: `IfNotPresent` (default, by digest so it's always safe), `Always` (re-check
  registry for tag→digest, only meaningful when not pre-resolved), `Never`.
- Option to disable digest pinning per workload for dev workflows.

## Private registries and credentials

- Pull secrets per pod/service account (like `imagePullSecrets`).
- **Credential providers**: plugins that mint short-lived tokens (ECR, GAR, ACR) –
  Kubernetes' kubelet credential provider exec plugins. Rustwain: a gRPC/exec plugin
  interface with response caching by registry+scope and expiry.
- **Ensure secret-pulled images**: Kubernetes had a long-standing issue where a pod
  *without* credentials could use an image already pulled by a pod *with* credentials
  (`IfNotPresent`). Fixed in recent Kubernetes (KEP-2535, "ensure secret pulled images").
  Rustwain records which credential (hash) authorised each image and re-verifies access
  for new pods with different credentials (a HEAD manifest request) – policy-controlled.

## Lazy pulling

Large images (ML: 5–20 GB) make cold starts slow; studies (e.g. the Slacker paper) show
containers read a small fraction (often < 10%) of image data at startup.

Options:
| Approach | Format change? | Notes |
|---|---|---|
| eStargz (stargz-snapshotter) | Yes (eStargz layers, still valid gzip) | FUSE, prefetch landmarks |
| SOCI (AWS) | No – separate index artifact | FUSE, index via referrers API |
| Nydus (Dragonfly) | Yes (RAFS) – or zran index for existing gzip | FUSE or EROFS+fscache in-kernel (6.x) |
| zstd:chunked (Podman) | Yes (zstd with TOC) | Partial pulls with file-level dedup, not lazy at runtime |

Decision `[proposed]`: design the snapshot trait to support a lazy backend, implement
**EROFS + fscache/on-demand** later (in-kernel, no FUSE in the IO path) using a
SOCI-like separate index so existing images need no rebuild. Not in milestone 1.

## Peer-to-peer distribution `[open]`

At DaemonSet scale, 1,000 nodes pulling a 2 GB image hammer the registry. Options:
- Node-local registry mirror/cache (simplest; per-zone pull-through cache).
- P2P (Dragonfly, Kraken, Spegel): nodes serve blobs they have to peers. Spegel's approach
  is attractive: each node advertises held digests via a DHT, and the runtime's mirror
  config points to localhost. Rustwain could build this into the agent: the image store
  already has content-addressed blobs; serving them over mTLS to peers is small. But it
  violates "agents don't talk to each other" (it's dataplane-ish, acceptable).

## Image volumes

Mount an OCI image/artifact (e.g. a model or a config bundle) read-only into a pod as a
volume (Kubernetes 1.31+ `image` volume source). Natural fit: it's just another snapshot
mounted read-only at a path. `[decided]`: supported from the first storage milestone.

## Platform selection details
- Match `os=linux`, `architecture` (`amd64`, `arm64`, `riscv64`), `variant` (`v8` for
  arm64 – treat missing variant as v8), optional `os.features`.
- Optional **compatibility attributes**: page size (images built for 4K pages), CPU
  feature level (`x86-64-v2/v3/v4`, `amd64` variants from OCI spec 1.1 annotations).
  Node attributes publish the host level; scheduling can match.
- Emulation opt-in: if a pod requests `allowEmulation`, and node has binfmt registered,
  pick a foreign-arch manifest.

## Open questions

- Store compressed blobs after unpack (for P2P serving and re-verification) or discard
  (saves ~30–50% image disk)? Default discard, configurable.
- Where to resolve tags to digests: API admission (needs registry access from servers)
  or controller (async)? Proposed: a dedicated image-resolver controller, so the API
  server never blocks on registries.

## Build checklist

- [ ] `rustwain-image` crate: refs parsing, registry client (auth flows, redirects,
      ranges, retries), content store, overlay snapshotter, metadata DB.
- [ ] Sandboxed unpacker binary mode (`rustwain unpack`) with fuzzing on tar inputs.
- [ ] Parallel layer pull+unpack benchmark vs containerd.
- [ ] Credential provider plugin interface.
- [ ] Image volume support.
