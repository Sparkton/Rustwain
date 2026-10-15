# Rust Ecosystem: What We Build On

Candidate crates per area, with the reason for choosing (or not). "Write ourselves" is a
legitimate choice for learning or control – marked explicitly. Versions intentionally
omitted; pin at implementation time.

## Async, networking, RPC
| Need | Choice | Notes |
|---|---|---|
| Async runtime | `tokio` | De facto standard; multi-thread runtime for server/agent |
| HTTP/2, HTTP/1 | `hyper`, `h2` | |
| gRPC | `tonic` + `prost` | Watch streams, plugin interfaces |
| Middleware | `tower` | Request pipeline layers |
| TLS | `rustls` (+ `aws-lc-rs` or `ring` provider) | aws-lc-rs for a FIPS path |
| Cert generation / parsing | `rcgen`, `x509-parser` / RustCrypto `x509-cert` | |
| HTTP client (registry, ACME) | `reqwest` (rustls) or `hyper-util` client | |
| QUIC/HTTP3 (later) | `quinn`, `h3` | Gateway |
| DNS | `hickory-proto`, `hickory-resolver` (+ server pieces) | Node-local resolver |
| L7 proxy (gateway) | `pingora` or own on hyper | `[open]` |

## Consensus and storage
| Need | Choice | Notes |
|---|---|---|
| Raft | `openraft` (lean) / `raft-rs` | See state store doc |
| Embedded KV (snapshots, agent checkpoint, image metadata) | `redb` | Pure Rust, ACID, simple |
| LSM option | `fjall` | If write-heavy needs arise |
| Persistent/immutable maps (MVCC snapshots) | `im` / `rpds` / own B-tree | `[open]` – performance-critical; may write our own |
| Serialization (internal) | `prost` (protobuf), `postcard` (compact, no_std), `serde` | |
| Compression | `zstd`, `flate2` (zlib-rs backend) | Images, snapshots |
| Object storage | `object_store` | Backups |

## Linux system interfaces
| Need | Choice | Notes |
|---|---|---|
| Syscalls | `rustix` | Safe wrappers incl. new mount API, pidfd, openat2 |
| Raw fallback | `libc` | Only where rustix lacks coverage (clone3 args) |
| Safe path resolution | `pathrs` (libpathrs) | Strong candidate for rootfs operations |
| Capabilities | `caps` | Or own on rustix/prctl |
| seccomp | `seccompiler` | Pure Rust BPF generation |
| Landlock | `landlock` | Official crate |
| Netlink | `netlink-packet-route`, `rtnetlink`, `netlink-sys`, `wireguard-uapi`/own genetlink | Evaluate vs own encoders |
| nftables | own netlink encoder (or `rustables`) | No shelling out to `nft` |
| eBPF | `aya`, `aya-ebpf`, `aya-log`; `bpf-linker` | Rust programs |
| D-Bus (systemd/logind) | `zbus` | Optional module |
| udev events | `netlink-sys` (KOBJECT_UEVENT) | Device hotplug |
| OCI types | `oci-spec` | Image/runtime spec types |
| Registry client | `oci-client` or own | Control over parallelism |
| Tar | `tar` | Sandboxed unpacker |
| Cgroups | own (`rustwain-cgroup`) | Reference: `cgroups-rs`, youki `libcgroups` |
| Container lib reference | youki `libcontainer` | Study / possibly reuse |
| KVM / VMM | Cloud Hypervisor (process); rust-vmm crates (`kvm-ioctls`, `vm-memory`, `linux-loader`, `virtio-queue`, `vhost`) for embedded VMM later | |
| vsock | `tokio-vsock` / `vsock` | Guest agent comms |

## API, policy, schema
| Need | Choice | Notes |
|---|---|---|
| JSON/YAML | `serde_json`, `serde_yaml` successor (`serde_yml` / `yaml-rust2`-based) | serde_yaml is archived – pick a maintained fork |
| JSON Schema / OpenAPI gen | `schemars` (+ custom extensions) | |
| CEL | `cel-interpreter` (cel-rust) | Evaluate maturity; contribute |
| Expression typing over Rust structs | own glue | |
| Kubernetes types (import adapter) | `k8s-openapi` | For parsing k8s manifests in `rwctl import` |
| Reference controller runtime | `kube-rs` (`kube-runtime`) | Design reference for our SDK |

## Security / supply chain
| Need | Choice |
|---|---|
| Sigstore verification | `sigstore` (sigstore-rs) |
| Hashing | `sha2`, `blake3` |
| AEAD | `aes-gcm` (RustCrypto) or via `aws-lc-rs` |
| Password/token hashing | `argon2` |
| Secrets in memory | `secrecy`, `zeroize` |

## CLI and UX
| Need | Choice |
|---|---|
| Args | `clap` (derive) |
| Tables/colour | `comfy-table` / `tabled`, `owo-colors`/`anstream` |
| Completion | `clap_complete` |
| OIDC login | `openidconnect` |
| Git (GitOps later) | `gix` |
| ACME | `instant-acme` |

## Testing and tooling
| Need | Choice |
|---|---|
| Property tests | `proptest` |
| Fuzzing | `cargo-fuzz` (libFuzzer), `arbitrary` |
| Deterministic simulation | `turmoil` / `madsim` / own harness |
| Benchmarks | `criterion` (or `divan`) |
| Snapshot tests | `insta` |
| Linearizability checking | port of porcupine (Rust ports exist) or own |
| Coverage | `cargo-llvm-cov` |
| Deps policy | `cargo-deny`, `cargo-audit`, `cargo-vet` |
| SBOM | `cargo-cyclonedx` |
| VM-based integration tests | Cloud Hypervisor/QEMU launched by `xtask` with cloud images |

## Observability hooks (deferred features, hooks now)
| Need | Choice |
|---|---|
| Logging/tracing | `tracing`, `tracing-subscriber` |
| Metrics facade | `metrics` |
| OTel (later) | `tracing-opentelemetry`, `opentelemetry-otlp` |

## Things we write ourselves (on purpose)
- cgroup v2 module, supervisor, container setup sequence (learning + minimal deps).
- Raft log storage, MVCC in-memory index, watch hub.
- Scheduler and controller runtime (inspired by kube-scheduler and kube-rs).
- Dataplane BPF programs.
- Node-local DNS zone logic.

## Open questions
- YAML library choice (ecosystem churn after serde_yaml archival).
- CEL completeness.
