# 0001: CI Pipeline

| Field | Value |
|-------|-------|
| Status | Accepted |
| Author | Akash |
| Created | 2026-10-17 |
| Depends on | none (references the design library: [testing strategy](../13-implementation/04-testing-strategy.md), [performance budgets](../11-efficiency/01-performance-budgets.md), [kernel feature matrix](../01-linux-foundations/07-kernel-feature-matrix.md)) |

## 1. Summary

Every push and pull request runs a GitHub Actions pipeline whose logic lives entirely in
`cargo xtask`. It checks:
- docs integrity (links, anchors, index, pain-point ID stability, decision markers);
- repository policy (Rust-only files, no AI attribution trailers, branch naming);
- Rust hygiene (fmt, clippy, tests, `cargo-deny`);
- on real Linux runners, the facts and latency floors the design depends on.

The latency part measures kernel primitive costs (namespace, cgroup and process
creation) and fails when they make a published budget infeasible. It is the first gate
that ties the performance budgets to measurement. The same gate framework later checks
component benchmarks against the budgets once code exists.

## 2. Motivation

- The design library is about 90 interlinked docs with stable pain-point IDs. Nothing
  enforces link integrity, index completeness or ID stability today; a manual check
  found and fixed drift during the second pass.
- The performance budgets (pod start 150 ms on-node, pod network setup 3 ms, …) are
  claims with no measurement behind them. Some depend on kernel costs we have not
  measured (netns creation is known to be slow on some kernels).
- The kernel feature matrix has many `[verify]` cells. CI runners are real Linux hosts
  that can answer some of them on every run.
- Project policy (Rust-only tooling, no AI attribution lines, `sparkton/` branches)
  is currently enforced by memory, not by machines.
- Starting CI before the first crate means every crate is born under fmt, clippy, deny
  and test gates instead of being retrofitted.

## 3. Kubernetes Today

From memory, pinned to upstream practice up to 2025 – verify before citing:
- **Prow + test-infra** run presubmits and periodics. Presubmit "verify" jobs are mostly
  `hack/verify-*.sh` shell scripts (gofmt, codegen up to date, boilerplate, vendor).
- **Scalability** is checked by `perf-tests`/ClusterLoader2 against published SLOs (for
  example pod startup latency and API call latency SLOs in `sig-scalability/slos`).
  These run as periodic jobs on 100-node and 5,000-node clusters, plus a 100-node
  performance presubmit.
- **Keep:** SLO-based performance gates; cheap verify checks on every PR; heavy scale
  tests as periodics, not presubmits.
- **Improve:** a per-PR latency *sanity* gate that is cheap and noise-tolerant;
  machine-checked links between docs, budgets and gates.
- **Drop:** shell-script verify jobs (policy: Rust only); Prow's operational weight
  (GitHub Actions is enough at this scale).

## 4. Goals and Non-Goals

Goals (each testable):
- **G1** Default PR pipeline wall-clock p50 < 5 min with a warm cache.
- **G2** `cargo xtask ci` runs the same non-Linux-specific checks locally on macOS and
  Linux with identical results.
- **G3** A broken relative link or anchor, an unindexed doc, a removed or duplicated
  pain-point ID, or an unknown decision marker fails CI with `file:line`.
- **G4** A non-Rust tooling file, a commit with an AI co-author/attribution trailer, or
  a PR branch outside `sparkton/` fails CI.
- **G5** fmt, clippy (`-D warnings`), tests and `cargo-deny` gate every PR as the
  workspace grows.
- **G6** Host facts (cgroup mode, PSI, BTF, LSMs, userns knobs, KVM) are recorded per
  runner as JSON artifacts and diffed against expectations.
- **G7** Kernel-floor latencies are measured on x86_64 and arm64 runners, and a floor
  that makes a published budget infeasible fails the run.
- **G8** Budgets exist once: `bench/budgets.toml` must agree with the budgets doc, and a
  drift fails CI.

Non-goals:
- External link checking (flaky, network-dependent).
- The distro and kernel VM matrix (nightly; needs `xtask vm-test`, its own design doc in
  M0).
- Self-hosted runners, release signing, artifact publishing.
- Dashboards or long-term metric storage (observability is deferred by project scope).
- Comparing Rustwain against Kubernetes in CI.

## 5. Proposal

### 5.1 Model

```rust
/// Every check xtask can run. Closed set → enum.
pub enum Check {
    DocLinks, DocAnchors, DocIndex, Markers,
    TrackerIds, TrackerSchema, BudgetSync,
    RustOnlyFiles, CommitTrailers, BranchName,
}

pub enum Severity { Error, Warning }

pub struct Location { pub path: PathBuf, pub line: NonZeroU32 }

pub struct Finding {
    pub check: Check,
    pub severity: Severity,
    pub location: Option<Location>,
    pub message: String,
}

pub struct Report { pub findings: Vec<Finding> }   // exit status non-zero iff any Error

/// Parsed once from disk; every docs check is a pure function over it.
pub struct DocSet { pub docs: BTreeMap<RelPath, Doc> }
pub struct Doc { pub headings: Vec<Heading>, pub links: Vec<Link>, pub markers: Vec<(Marker, NonZeroU32)> }
pub enum Marker { Proposed, Decided, Open, Later, Skip, Verify }

/// `PP-[0-9A-F][0-9A-Z]`, `PP-R<n>` or `PP-W<n>`; validated on parse.
pub struct PainPointId(String);
pub enum PainPointStatus { Designed, Building, Done, Deferred, WontFix, Open }
pub struct TrackerRow { pub id: PainPointId, pub status: Option<PainPointStatus>, pub line: NonZeroU32 }

/// Host facts – the subset of the kernel feature matrix probes that need no product code.
pub struct HostFacts {
    pub schema: u32,
    pub kernel_release: String,
    pub arch: Arch,
    pub os_release: OsRelease,               // ID, VERSION_ID, ID_LIKE
    pub cgroup: CgroupMode,
    pub cgroup_controllers: Vec<String>,
    pub psi: PsiState,
    pub btf: BtfState,
    pub lsms: Vec<String>,
    pub userns: UsernsKnobs,                 // apparmor_restrict, unprivileged_clone, max_user_namespaces
    pub kvm: KvmState,
    pub lockdown: Option<Lockdown>,
}
pub enum CgroupMode { Unified, Hybrid, Legacy }
pub enum PsiState { Enabled, DisabledAtBoot, NotCompiled }
pub enum BtfState { Present, Absent }
pub enum KvmState { Usable, PresentNoAccess, Absent }

/// Latency floors.
pub enum FloorMetric { NetnsCreate, UsernsCreateMap, CgroupLifecycle, SpawnWait, MountNsTmpfs }
pub struct Summary { pub metric: FloorMetric, pub n: u32, pub p50: Duration, pub p90: Duration, pub p99: Duration, pub max: Duration }

pub struct BudgetFile { pub budgets: Vec<Budget>, pub floors: Vec<FloorRule> }
pub struct Budget { pub id: BudgetId, pub doc_row: String, pub p50: Duration, pub p99: Duration }
pub struct FloorRule { pub metric: FloorMetric, pub feeds: BudgetId, pub max_share_p50: Ratio }

pub enum GateOutcome {
    Pass,
    NoBaseline,
    Regressed { baseline: Summary, current: Summary },
    BudgetInfeasible { budget: BudgetId, floor_p50_sum: Duration, allowed: Duration },
}
pub enum GateMode { Advisory, Blocking }
```

### 5.2 Ownership and Concurrency

- `xtask` is a synchronous, single-process CLI. No async runtime.
- A thin loader owns all file I/O and builds `DocSet`, `BudgetFile` and the tracker
  rows once. Each check is a pure function (`fn(&DocSet) -> Vec<Finding>`) and is
  unit-testable without a filesystem.
- Floor benchmarks run each sample on a fresh thread or forked child, because
  `unshare` changes the calling thread's (or process's) namespaces. Samples go into a
  pre-sized `Vec`; nothing is shared between samples.
- `git` is invoked as a subprocess (`std::process::Command`) for commit messages and
  branch names. This is tooling, not product, so the product's no-shell-out rule doesn't
  apply. It avoids pulling `gix` into every CI build.

### 5.3 Interfaces

CLI (`cargo xtask …`, via a `.cargo/config.toml` alias):

| Command | Purpose | Platforms |
|---|---|---|
| `ci` | Runs `docs check`, `policy check`, fmt, clippy and test – the local equivalent of the blocking jobs | all |
| `docs check` | Links, anchors, index, markers, tracker, budget sync | all |
| `docs report` | Counts of `[verify]`, `[open]`, `[proposed]` per doc → markdown (job summary) | all |
| `docs lock-ids` | Appends new pain-point IDs to the lockfile (explicit, reviewed in the diff) | all |
| `policy check [--range <base>..<head>]` | Rust-only files, commit trailers, branch name | all |
| `host-facts [--json <path>]` | Probe and print `HostFacts`; diff against expectations | Linux |
| `bench floor [--iterations N] [--json <path>]` | Measure kernel floors (needs root) | Linux |
| `bench gate --current <json> [--baseline <json>]` | Apply `budgets.toml` rules → `GateOutcome`s | all |

Files:

| Path | Format | Owner |
|---|---|---|
| `.github/workflows/ci.yml` | Workflow; every step is a single `cargo xtask …` call or a pinned action | xtask design |
| `rust-toolchain.toml` | Pinned stable toolchain | workspace |
| `deny.toml` | Advisories, licences, sources, bans (`im`, `serde_yml`, `openssl-sys`, `rocksdb`, `aws-lc-fips-sys`) | workspace |
| `bench/budgets.toml` | `[[budget]]` and `[[floor]]` tables; each budget names its row in the budgets doc | performance budgets doc |
| `ci/host-expectations.toml` | Expected host facts per `(ID, VERSION_ID, arch)` | kernel feature matrix |
| `docs/12-critique/pain-point-ids.lock` | One ID per line, append-only | tracker |
| JSON outputs | `{"schema": 1, …}` – versioned; the schema bump is part of any breaking change | xtask |

Job summaries are written by xtask to the file named in `GITHUB_STEP_SUMMARY`, so the
workflow needs no shell logic.

### 5.4 Behavior

Jobs in `ci.yml`:

| Job | Runner | Trigger | Mode | Runs |
|---|---|---|---|---|
| `policy` | `ubuntu-24.04` | push, PR | Blocking | `policy check --range` over the PR's commits |
| `docs` | `ubuntu-24.04` | push, PR | Blocking | `docs check`; `docs report` → summary |
| `rust` | `ubuntu-24.04` | push, PR | Blocking | fmt `--check`, clippy `--workspace --all-targets -D warnings`, test `--workspace` |
| `rust-macos` | `macos-14` (arm64) | push to `main`; PRs labelled `macos` | Blocking | build + test: the primary dev platform must stay green |
| `deny` | `ubuntu-24.04` | PRs touching `Cargo.lock`/`deny.toml`; weekly | Blocking | `cargo deny check` |
| `host-facts` | `ubuntu-24.04`, `ubuntu-24.04-arm` | push, PR | Advisory | `host-facts --json` (unprivileged, then with `sudo` for root-only probes) → artifact + diff warnings |
| `kernel-floor` | `ubuntu-24.04`, `ubuntu-24.04-arm` | push, PR | `BudgetInfeasible` blocking; `Regressed` advisory | `sudo … bench floor` → `bench gate` against the cached `main` baseline |

Flow of a PR run:

1. Checkout, toolchain from `rust-toolchain.toml`, `Swatinem/rust-cache` keyed on
   `Cargo.lock`.
2. Build xtask once per job (cached), then run the job's `cargo xtask` command.
3. `policy`, `docs`, `rust` and `deny` produce `Finding`s. Any `Error` fails the job,
   and every finding becomes a GitHub annotation at its `file:line`.
4. `host-facts` uploads its JSON and annotates differences from the expectations as
   warnings. A difference is information (the runner image changed, or a `[verify]` cell
   was wrong), never a failure.
5. `kernel-floor` runs 20 warm-up and 200 measured iterations per metric and computes
   p50/p90/p99/max:
   - **Feasibility (blocking):** for each budget, the sum of its feeding floors' p50 must
     be ≤ `max_share_p50` × the budget's p50. Initial rules:

     | Floor metric | Operation | Feeds | Max share of budget p50 |
     |---|---|---|---|
     | `NetnsCreate` | `unshare(CLONE_NEWNET)` on a fresh thread | Pod network setup (3 ms) | 33% |
     | `UsernsCreateMap` | forked child: `unshare(CLONE_NEWUSER)` + write a 65,536-ID `uid_map`/`gid_map` | Pod start, process class (150 ms) | combined 25% |
     | `CgroupLifecycle` | `mkdir` pod cgroup, write `cpu.weight`, `memory.max`, `pids.max`, `rmdir` | Pod start | (in the 25%) |
     | `SpawnWait` | spawn `/bin/true` and wait | Pod start | (in the 25%) |
     | `MountNsTmpfs` | `unshare(CLONE_NEWNS)`, private propagation, mount + unmount tmpfs | Pod start | (in the 25%) |

     The shares are deliberately generous: they leave room for everything Rustwain adds
     on top. A failure means the *budget assumption* is wrong on that kernel – change
     the design (for example, a pre-created netns pool) or the budget, and record it in
     the budgets doc.
   - **Regression (advisory):** compare with the most recent `main` baseline restored
     from the Actions cache (`kernel-floor-<os>-<arch>-*`). Flag p50 increases of
     > 50% *and* > 50 µs absolute. Hosted runners are shared VMs, so this stays advisory
     until a dedicated runner exists. Pushes to `main` save a new baseline.
6. `docs check` also verifies `bench/budgets.toml` against the budgets doc. Each
   `doc_row` must exist and its `p50 / p99` text must equal the TOML values.
7. Later, without changing the pipeline shape, component benches (`xtask bench <suite>`)
   emit the same `Summary` JSON and go through the same gate. Absolute budget gates for
   components become blocking only on a dedicated runner (see Open Questions).

## 6. Failure Modes and Recovery

| Failure | Detection | Effect | Recovery |
|---|---|---|---|
| Timing noise on shared runners | Wide p90–p99 spread; flapping advisory results | None on merge (regression is advisory; feasibility has large margins) | Raise iterations; move blocking regression gates to a dedicated runner |
| Runner image or kernel changes | `host-facts` diff; floor baseline shift | Advisory warnings | Update `ci/host-expectations.toml`; the new baseline is saved on the next `main` push |
| No baseline in cache (first run, eviction) | Cache miss | `NoBaseline` → pass with a notice | Next `main` push saves one |
| `sudo` or a privileged syscall unavailable | `EPERM` from the probe | Metric reported as skipped (`Warning`), not failed | Fix the runner, or keep advisory |
| xtask bug produces false failures | Developer report | Blocks merges | `--skip <check>` with a mandatory reason, printed in the summary; fix forward |
| GitHub Actions outage | No runs | No CI signal | `cargo xtask ci` locally gives the same blocking results |
| Lockfile not updated for new pain-point IDs | `TrackerIds` error | PR blocked | `cargo xtask docs lock-ids`, then commit the lockfile diff |
| Fork PR | Workflow runs without secrets | None: no secrets are used | — |

## 7. Scalability and Performance

- `docs check` over ~100 docs: target < 2 s after build (it is linear in total doc size).
- xtask cold build: < 60 s; dependencies limited to `clap`, `serde`, `toml`,
  `serde_json`, plus `rustix` on Linux. Warm cache: < 5 s.
- `kernel-floor`: ~1,100 samples per runner; target < 60 s per job.
- Default PR pipeline wall-clock (jobs in parallel): p50 < 5 min (G1), measured from the
  Actions run timings over the first 20 PRs.

## 8. Security

- Workflow `permissions: contents: read` by default. Only the cache steps write, using
  the built-in cache scope.
- Third-party actions pinned by commit SHA (`actions/checkout`,
  `actions/upload-artifact`, `actions/cache`, `Swatinem/rust-cache`,
  `taiki-e/install-action` for `cargo-deny`). Dependabot or a scheduled job proposes
  updates.
- No `pull_request_target`; no secrets; `sudo` only on ephemeral GitHub-hosted runners.
  Privileged jobs never run on self-hosted runners for fork PRs.
- xtask reads only repository files and `/proc`, `/sys` on the runner. It writes only
  under `target/` and the step-summary file.

## 9. Alternatives Considered

| Alternative | Why not |
|---|---|
| Shell/Python verify scripts (Kubernetes' `hack/verify-*`) | Violates the Rust-only policy |
| `lychee` (a Rust link checker) for links | Handles links, but not anchors-by-GitHub-slug, index completeness, tracker IDs or budget sync, which need custom logic anyway. Could be added later for *external* links |
| Blocking `criterion` wall-clock gates on hosted runners | Too noisy for per-PR blocking. Feasibility checks with large margins plus advisory regressions are the honest middle |
| Instruction-count benchmarks (`iai-callgrind`) | Stable on noisy runners, but needs valgrind, an external C tool. Revisit for hot-path microbenchmarks on a dedicated runner |
| `gix` for commit inspection | Pure Rust, but a heavy dependency for a two-command need |
| Prow | Operationally heavy for a single-maintainer project |

## 10. Testing and Verification Strategy

- **Unit tests** for every check over in-memory `DocSet` fixtures: broken link, missing
  anchor, unindexed doc, duplicate and removed IDs, unknown marker, budget drift. One
  passing and one failing fixture each.
- **Slug tests:** a table of heading → anchor cases matching GitHub's rules (en dashes,
  backticks, brackets, duplicates `-1`).
- **Gate logic:** pure-function tests over synthetic `Summary` values for every
  `GateOutcome`, including boundary values.
- **Host-facts parsers:** tested on captured `/proc` and `/sys` file fixtures, so they
  run on macOS too.
- **Self-check:** CI runs `cargo xtask docs check` on the repository itself – the
  library is the integration test.

## 11. Implementation Plan

Each ticket is one 3-1 cell (three Sonnet implementers on disjoint slices, one
orchestrator that writes only the skeleton).

**T1 – xtask skeleton, docs and policy checks, blocking jobs.**
- Skeleton (orchestrator): workspace `Cargo.toml` with `xtask` as the only member,
  `.cargo/config.toml` alias, `rust-toolchain.toml`, `clap` command enum, the `Check`,
  `Finding` and `Report` types and the check function signatures.
- Slice A: `DocSet` loader; links, anchors, index and markers checks; fixtures.
- Slice B: tracker parsing, lockfile, `TrackerIds`/`TrackerSchema`; policy checks.
- Slice C: `ci.yml` (`policy`, `docs`, `rust`, `rust-macos`, `deny` jobs), `deny.toml`,
  the `ci` aggregator, the step-summary writer and annotations.
- Acceptance: `cargo xtask ci` exits 0 on the branch, on macOS and Linux. Each G3/G4
  violation, introduced in a fixture, fails with the right `file:line`. The PR's own CI
  run is green.

**T2 – host facts, kernel floors, budget gate.**
- Slice A: `HostFacts` probes and `ci/host-expectations.toml` diff.
- Slice B: floor benchmarks (`rustix` on Linux), `Summary` JSON.
- Slice C: `bench/budgets.toml`, the budget-sync check, gate logic, the `host-facts` and
  `kernel-floor` jobs, and baseline caching.
- Acceptance: JSON artifacts from both runner architectures. A synthetic infeasible
  budget fails the job. Editing a budget in the doc without `budgets.toml` fails
  `docs check`.

**T3 – nightly VM and kernel matrix** – separate design doc, after `xtask vm-test` (M0).

## 12. Open Questions

- Is the GitHub repository public? macOS runner minutes are expensive on private repos;
  that decides whether `rust-macos` runs on every PR or only on `main`.
- Do GitHub-hosted x86_64 runners expose a usable `/dev/kvm` today? `host-facts`
  answers this on its first run. It decides whether the nightly VM matrix can run on
  hosted runners.
- Dedicated benchmark runner (bare metal, fixed kernel): needed before absolute
  component budgets block merges. Owner: performance budgets doc, at M1.
- Should `BranchName` apply to every PR, or only to branches pushed by agents? Policy
  says all pushed branches; confirm.

## 13. Decision Log

- 2026-10-17: Accepted by Akash as written. Ticket T1 starts against this version.
