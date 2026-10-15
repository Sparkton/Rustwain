# Crate Layout

A Cargo workspace with strict dependency direction. The goal: security-critical and
low-level crates have minimal dependencies; high-level crates can't be accidentally
imported by low-level ones.

## Workspace

```
rustwain/
├─ Cargo.toml                    (workspace, shared lints, profiles)
├─ rust-toolchain.toml
├─ xtask/                        (cargo xtask: build, test matrix, bench, guest kernel, release)
├─ crates/
│  ├─ sys/                       LOW LEVEL – unsafe allowed here only
│  │  ├─ rustwain-sys            syscalls wrappers: clone3, pidfd, mount API, openat2, caps, prctl
│  │  ├─ rustwain-cgroup         cgroup v2 typed controllers
│  │  ├─ rustwain-netlink        rtnetlink/genetlink/nftables encoders (or wrappers over netlink crates)
│  │  ├─ rustwain-seccomp        profiles + compilation (seccompiler)
│  │  └─ rustwain-lsm            SELinux/AppArmor/Landlock backends
│  │
│  ├─ node/                      NODE SIDE
│  │  ├─ rustwain-probe          host feature detection + doctor checks
│  │  ├─ rustwain-container      namespace/rootfs/exec setup (phases 1–4); used by supervisor and guest
│  │  ├─ rustwain-supervisor     per-pod supervisor (sync, no tokio)
│  │  ├─ rustwain-init           tiny PID 1 (no_std-ish, static)
│  │  ├─ rustwain-image          registry client, content store, snapshotters, unpacker
│  │  ├─ rustwain-runtime        RuntimeBackend trait + native/microvm backends
│  │  ├─ rustwain-vmm            Vmm trait + Cloud Hypervisor driver
│  │  ├─ rustwain-guest          in-VM agent
│  │  ├─ rustwain-bpf            eBPF programs (no_std, bpf target) – built by xtask
│  │  ├─ rustwain-dataplane      BPF loading/maps, link mgmt, IPAM local, nft fallback, WireGuard
│  │  ├─ rustwain-dns            node-local resolver
│  │  ├─ rustwain-volumes        volume manager, CSI node client, local driver
│  │  ├─ rustwain-devices        device drivers, CDI
│  │  └─ rustwain-agent          composition: pod manager, eviction, probes, status, tunnel
│  │
│  ├─ control/                   CONTROL PLANE
│  │  ├─ rustwain-store          Raft + MVCC + watch source + snapshots
│  │  ├─ rustwain-api-server     request pipeline, authn/z, admission, fairness, gateway
│  │  ├─ rustwain-policy         CEL evaluation, ValidationPolicy/MutationPolicy
│  │  ├─ rustwain-scheduler      framework + plugins + simulation API
│  │  ├─ rustwain-controllers    built-in controllers
│  │  ├─ rustwain-pki            CA, issuance, rotation
│  │  └─ rustwain-server         composition binary logic
│  │
│  ├─ shared/                    SHARED (no I/O-heavy deps)
│  │  ├─ rustwain-api-types      all API kinds, metadata, validation derive output
│  │  ├─ rustwain-api-derive     proc macros (validation, list semantics, schema, client gen)
│  │  ├─ rustwain-proto          generated protobuf + conversions
│  │  ├─ rustwain-client         API client, reflectors, watch, typed clients
│  │  ├─ rustwain-controller-runtime  public SDK
│  │  ├─ rustwain-units          Bytes, MilliCpu, quantities parsing
│  │  └─ rustwain-testkit        in-memory API, simulated clock, fake agents
│  │
│  └─ cli/
│     └─ rwctl
└─ bins/
   └─ rustwain                   single binary: dispatches `server`, `agent`, `supervise`, `dev`, `doctor`, ...
```

## Dependency rules (enforced)
1. `sys/*` depend only on `rustix`, `libc` (if unavoidable), `bitflags`, `thiserror`.
2. `rustwain-container`, `rustwain-supervisor`, `rustwain-init`: **no tokio**, no async,
   minimal deps – they run in fork/exec contexts and must be tiny.
3. `shared/*` never depend on `node/*` or `control/*`.
4. `node/*` never depend on `control/*` (and vice versa), except via `shared/*`.
5. Only `sys/*` (and explicitly listed modules in `node/*`, e.g. dataplane loader) may use
   `unsafe`. Enforced with `#![forbid(unsafe_code)]` in all other crates.
6. Enforcement via `cargo xtask check-deps` (parses `cargo metadata`) in CI + `cargo-deny`
   bans.

## Lints and profiles
- Workspace lints: `unsafe_op_in_unsafe_fn = deny`, `clippy::unwrap_used = deny` (non-test),
  `clippy::expect_used = warn`, `missing_docs` for public SDK crates.
- Release profile: `lto = "fat"`, `codegen-units = 1`, `panic = "unwind"` for agent/server
  (module supervision), `panic = "abort"` for supervisor/init (smaller, simpler) – requires
  separate binaries or careful configuration (`[open]`: same-binary supervisor can't have a
  different panic strategy → argues for separate supervisor binary).
- Size: `opt-level = "s"` for supervisor/init/guest.

## Feature flags
- Minimal: `microvm`, `nftables-fallback`, `selinux`, `apparmor` features to trim builds
  for constrained targets. Default build includes everything.

## Open questions
- Single `rustwain` binary vs separate `rustwain-supervisor` (panic strategy, size).
- Use existing netlink crates (`rtnetlink`, `netlink-packet-*`, `wireguard-uapi`) vs
  own encoders.

## Build checklist
- [ ] Workspace skeleton with crates above (empty, compiling).
- [ ] `xtask check-deps` enforcing rules.
- [ ] CI: fmt, clippy, test, deny, audit.
