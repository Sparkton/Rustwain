# Design Review: Assumptions, Shortcomings, Plan Verdict

A review of the design library as of 2026-10-17. It covers three things:
- which assumptions were checked against current sources, and what changed as a result;
- which internal inconsistencies or design flaws were found and fixed;
- which risks remain open, and whether the implementation plan makes sense.

Re-run this kind of review at the end of every milestone. New rows go in the tables;
nothing is deleted.

**Coverage.** Read in full: overview, implementation, state store, watch, scheduler, API
server, object model, fairness, node agent (architecture, lifecycle, eviction,
registration), networking (model, pod networking, dataplane, services, IPAM), runtime
architecture, process supervision, cgroups (layout and systemd sections), microVMs,
storage model, PKI, upgrades, HA, budgets. Other docs were scanned for version-specific
claims only. Workloads, DNS, policy, mesh, devices, images and most security docs were
not reviewed in depth and should be next.

## Verdict on the implementation plan

**The plan makes sense, with five corrections now applied:**
- The **bottom-up order is right.** Node primitives → single-node runtime → node
  networking → control plane → multi-node follows the real dependency chain, and every
  milestone can be tested alone (`agent --standalone`, store as a library, API on an
  in-memory store).
- **Embedding the store** removes etcd's operational burden and the watch-cache double
  memory. The sizing problem found below is fixable and has been fixed in the design.
- **Corrections applied:**
  1. **Design gate.** Project policy requires an accepted `docs/design/NNNN` doc before
     any code; the roadmap now says so explicitly, and the first such doc exists
     ([0001 – CI pipeline](../design/0001-ci-pipeline.md)).
  2. **M1 was too large** for one step; it is now M1a (process sandbox from a local
     rootfs) → M1b (images) → M1c (pod features).
  3. **The nftables fallback was in M2 for the wrong reason** (missing BTF). The core
     dataplane needs no kernel BTF, so the fallback moves to M7.
  4. **No macOS development loop** existed, yet the primary dev machine is a Mac. M0 now
     includes a Linux dev VM and states the M6 KVM-on-Mac constraint.
  5. **No CI existed.** Design 0001 proposes it (awaiting acceptance).
- **Two recommendations not yet applied** (they change scope, so they need your call):
  - **Run M3 (store + API) as a parallel track to M1/M2.** Nothing in M3 depends on the
    runtime until M4 joins them, and the 3-1 workflow can staff two tracks.
  - **Time-box four risk spikes before the decisions they could invalidate** (each is
    small and can overturn a big decision):

    | Spike | Validates | Before |
    |---|---|---|
    | Supervisor PSS with a separate small binary | < 1 MB/pod budget; separate-binary decision | M1a design doc |
    | openraft + deterministic simulation harness (`madsim` or `turmoil`) driving 3 nodes with partitions | That an async Raft library can run under our simulator at all | M3 design doc |
    | Aya program (TC + `cgroup/connect6`) loaded on 5.15, 6.1, 6.12 kernels | Rust-emitted BPF passes the verifier on the floor kernel | M2 design doc |
    | Lease failover + self-fencing protocol in a Rust model checker (`stateright`) | No double-running singleton pods under failover | M4 design doc |

- **Scope honesty.** M0–M7 is many engineer-years of work. Define "minimum useful
  cluster" as M0–M4 with the `process` isolation class only. Treat everything after that
  as optional depth. This matches the learning goal and keeps the project shippable.

## A. Assumptions checked against current sources

| # | Assumption in the docs | Verdict | Evidence | Docs changed |
|---|---|---|---|---|
| A1 | Ubuntu 26.04 ships "6.17+" | **Corrected:** ships Linux **7.0** (final) and systemd 259 | [Canonical kernel team](https://discourse.ubuntu.com/t/26-04-lts-resolute-raccoon-shipping-with-the-final-7-0-linux-kernel/80838), [release notes summary](https://linuxiac.com/ubuntu-26-04-lts-resolute-raccoon-released/) | host variance, kernel matrix |
| A2 | `im` is a reasonable persistent-map crate | **Corrected:** unmaintained, archived 2026-05; use `imbl` | [RUSTSEC-2026-0248](https://rustsec.org/advisories/RUSTSEC-2026-0248) | ecosystem, state store, scheduler sketch |
| A3 | `serde_yml` is a valid `serde_yaml` successor | **Corrected:** unsound and archived; use `serde_norway` or `serde_yaml_ng` | [RUSTSEC-2025-0068](https://rustsec.org/advisories/RUSTSEC-2025-0068) | ecosystem |
| A4 | aws-lc-rs gives an easy FIPS path | **Corrected:** FIPS builds need CMake **and Go**, which breaks the Rust-only tooling rule. FIPS is out of scope | [aws-lc-rs requirements](https://aws.github.io/aws-lc-rs/requirements/index.html) | ecosystem, PKI |
| A5 | kube-scheduler scores ~5% of nodes at 5,000 | **Corrected:** ~10% at 5,000, 5% floor, ≥ 100 feasible nodes | [Scheduler performance tuning](https://kubernetes.io/docs/concepts/scheduling-eviction/scheduler-perf-tuning/) | scheduler |
| A6 | Aya lets our Rust BPF programs use BTF/CO-RE, so the dataplane needs kernel BTF | **Corrected:** CO-RE *emission* from Rust is experimental (bpf-linker) and the compiler feature is only a pre-RFC. Our programs therefore use UAPI contexts only and need no kernel BTF | [Rust pre-RFC: BTF relocations](https://internals.rust-lang.org/t/pre-rfc-btf-relocations/24161), [Aya CO-RE example](https://codeberg.org/vad_sol/aya-core-relocations) | dataplane, host variance, kernel matrix, crate layout, roadmap |
| A7 | openraft supports ReadIndex and leader-lease reads | **Confirmed**, with a caveat: the 0.10 API (`ensure_linearizable(ReadPolicy)`) is still alpha, and 0.9 differs | [openraft read protocol docs](https://docs.rs/crate/openraft/0.9.25/source/src/docs/protocol/read.md) | state store |
| A8 | Alpine/OpenRC cgroup default unclear | **Confirmed unified** by default since OpenRC 0.51. One report shows first-boot provisioning forcing hybrid, so the probe stays authoritative | [Alpine wiki: OpenRC](https://wiki.alpinelinux.org/wiki/Openrc) | kernel matrix |
| A9 | macOS can host the full dev loop | **Partly:** nested virtualization exists on M3+ Macs with macOS 15+, but KVM inside a Linux guest is unconfirmed `[verify]` | [summary of Apple's API change](https://news.hada.io/topic?id=15318) | roadmap M0, testing strategy |
| A10 | CI can run VM tests on hosted runners | **Unconfirmed:** hosted arm64 runners likely lack `/dev/kvm`; x86_64 `[verify]` (design 0001's `host-facts` answers it) | [runner-images discussion](https://github.com/actions/runner-images/discussions/7191) | testing strategy, design 0001 |
| A11 | PSI is disabled by default on Debian and RHEL kernels | **Still unverified.** No authoritative source found either way. Stays `[verify]`; needs the distro VM matrix (Ubuntu runners can't answer it) | [kernel PSI docs](https://kernel.org/doc/html/v6.6/accounting/psi.html) (mechanism only) | none |
| A12 | Kubernetes version facts (nftables GA 1.33, sidecars, DRA GA 1.34, userns gates, kubelet N−3 skew) | Spot-checked; consistent | — | none |

## B. Shortcomings found and fixed

| # | Problem | Fix | Docs changed |
|---|---|---|---|
| B1 | **systemd cannot delegate a slice.** The layout relied on a "delegated `rustwain.slice`" and on `KillMode=process` for the agent | Pods live under a dedicated delegated unit, `rustwain-pods.service` (with a tiny keeper process). The agent keeps the default `KillMode=control-group`. New pain point PP-5H | cgroups v2, host variance, bootstrap, process supervision, tracker |
| B2 | **Store memory didn't add up.** The live-store cap (8 GiB) equalled the per-server RSS target (8 GB), and a time-bounded history (10 min at 10k writes/s ≈ 24 GB) can't fit | Live-bytes cap 4 GiB; in-memory history bounded by bytes (512 MiB); older history served from an on-disk change journal; a new M3 exit criterion measures RSS | state store, roadmap |
| B3 | Leader-local leases imply the node lifecycle logic must run on the Raft leader, but nothing said so (controllers are spread across servers) | Node lifecycle is a leader-pinned task fenced by the Raft term | state store, architecture |
| B4 | Follower-originated writes could wait a full heartbeat (250 ms default) to learn commit | Requirement: the leader broadcasts commit as soon as it advances, with a test | state store |
| B5 | The second-pass crash walkthrough contradicted the HA doc on who restarts containers while the agent is down | One decision-maker at a time: the supervisor restarts after 5 s of disconnection using a shared pure function, then hands control back. New PP-R6 for the cost | pod lifecycle, HA, tracker |
| B6 | Walkthrough used a different supervisor socket path and an in-memory event buffer; process-supervision doc uses `pods/<uid>/ctl.sock` and a state file | Aligned to the process-supervision doc | pod lifecycle |
| B7 | Crash points named differently from the testing strategy's failpoints | Uses `fail` crate failpoints and the `vm-test` runner | pod lifecycle |
| B8 | **MicroVM TAP placement contradicted itself** (in the pod netns vs host side) | TAP in the host netns, its fd passed to the VMM, VMM in an empty netns (smaller attack surface, one hop fewer) | microVMs |
| B9 | Architecture said "built-in rust-vmm VMM"; roadmap and crates say Cloud Hypervisor first | Aligned on Cloud Hypervisor first | architecture, runtime |
| B10 | "Agents only watch objects bound to themselves" is false: every agent needs cluster-wide services, endpoints and identities | Stated explicitly, noting that it is O(cluster) per node and must be sliced and delta-encoded | architecture |
| B11 | One `rustwain` binary per supervisor conflicts with the < 1 MB per-pod budget and the panic strategy | Separate `rustwain-supervisor` binary `[proposed]`, to be confirmed by the M1 PSS measurement | crate layout, process supervision, bootstrap |
| B12 | BPF builds need nightly + `bpf-linker`; undocumented | Toolchains section; BPF crate isolated from the stable workspace build | crate layout, dataplane |
| B13 | Vision (< 300 ms) and budgets (150 ms on-node / 400 ms end-to-end) gave different pod-start targets | Vision now points to the budgets | vision |
| B14 | Garbled immutability example in the object model | Rewritten | object model |
| B15 | No CI, no machine checks of docs, budgets or policy | Design 0001 proposed | design/0001, testing strategy, roadmap |

## C. Open risks (not fixed – need a decision or a measurement)

| # | Risk | Why it matters | Next step |
|---|---|---|---|
| C1 | PSI default-disabled on Debian/RHEL (A11) | Eviction's headline feature degrades on two tier-1 families | Distro VM matrix in M0; decide `degraded` vs refuse (PP-R1) |
| C2 | 10k writes/s with 3 replicas and fsync | ~3–5× typical etcd numbers; group commit makes it plausible on NVMe, but it is unmeasured | Store benchmark in the M3 spike |
| C3 | Deterministic simulation of an async (tokio) Raft library | The store's correctness strategy depends on it; openraft is not built for `madsim`/`turmoil` by default | openraft + simulator spike (plan verdict) |
| C4 | Endpoint fan-out at 5k nodes (p99 < 1 s, delta watch `[proposed]`) | Dominant control-plane cost during rollouts | Fake-agent scale test in M4; decide delta watch with data |
| C5 | Rust BPF verifier friction on the 5.15 floor | Could force program restructuring or a higher kernel floor | Aya spike (plan verdict); per-kernel load tests in CI later |
| C6 | CEL implementation maturity in Rust | Admission policy design depends on it | Evaluate the `cel` crate at M7 design time |
| C7 | Leader-local lease safety proof | Registration doc lists it as open; a mistake means double-running singletons | Model-check with `stateright` before M4 |
| C8 | Project scope vs one engineer | Risk of never reaching a usable cluster | "Minimum useful cluster" = M0–M4, process class only |
| C9 | DSR needs cloud anti-spoofing off (PP-R2) | Limits DSR to bare metal and configured clouds | Already designed with probe + SNAT fallback |

## Process note

There are two documentation layers, and the policy treats them differently:
- the **design library** (`docs/<NN-area>/`, `[proposed]`/`[decided]` markers):
  research and exploration;
- **numbered design docs** (`docs/design/NNNN-*.md`, Draft → In Review → Accepted by the
  user): implementation contracts for the 3-1 workflow.

The design-doc index (`docs/design/README.md`) and the agent policy live on the
`sparkton/agent-skills-and-policy` branch. When that branch merges, add the row for 0001
to its index.
