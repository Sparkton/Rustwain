# Design Principles

These principles exist so that design arguments can be settled by pointing at a rule
rather than relitigated each time. If a principle turns out to be wrong, change it here
first.

## 1. Level-triggered, not edge-triggered

Every controller computes "what should be" from the *current* full state and acts on
the difference. Events are hints that something changed, never the source of truth.

- Consequence: missing an event is harmless; a periodic resync fixes it.
- Consequence: controllers can restart at any time.
- This is the single best idea in Kubernetes. We keep it unchanged.

## 2. The API is the only coordination bus

Components coordinate by reading and writing objects. No hidden side channels between
controllers, no agent-to-agent control traffic. Everything the system is doing is
inspectable with `rwctl get`.

## 3. Fewer processes, deeper modules

Kubernetes splits a node across 6–10 processes for organisational reasons. We split only
where there is a *technical* reason:

| Boundary | Technical reason it exists |
|---|---|
| server ↔ agent | Different machines |
| agent ↔ supervisor | Workloads must survive agent restarts and crashes |
| supervisor ↔ VMM/guest | Hardware isolation boundary |

Everything else (runtime, CNI, kube-proxy, DNS cache, CSI node logic) is a module inside
the agent with a Rust trait boundary, not a process boundary.

## 4. Typed all the way down

- Resource schemas are Rust types. Protobuf/JSON schemas and OpenAPI docs are generated
  from them, never hand-written.
- Invalid states should be unrepresentable where practical: enums over strings,
  newtypes for IDs (`NodeName`, `PodUid`), `NonZeroU32` for counts that must be positive.
- Units are types: `Bytes`, `MilliCpu`, `Duration` – no `"500m"` strings past the parsing
  edge.

## 5. Fail closed, degrade gracefully

- Security decisions fail closed: if policy can't be evaluated, deny.
- Availability decisions degrade gracefully: if the API is unreachable, agents keep
  running existing workloads, keep the dataplane up, and keep restarting crashed
  containers per policy. They never tear down workloads just because they lost contact.

## 6. Explicit over implicit defaults

Kubernetes has many silent defaults that surprise people (ndots:5, no resource requests,
`imagePullPolicy` depending on tag, `automountServiceAccountToken: true`). Rustwain:

- Shows the fully defaulted object on `rwctl get -o full`.
- Prefers secure defaults (no API token mounted unless requested, user namespaces on,
  read-only root fs suggested by lint).
- Warns at apply time on dangerous combinations rather than silently accepting them.

## 7. O(changes), not O(cluster)

No component should do work proportional to cluster size on every change. Examples:

- Service updates touch only the affected eBPF map entries (not a full iptables rewrite).
- Agents watch only their own pods, not all pods.
- Endpoint updates are sliced and diffed.
- Controllers use indexed caches, not list scans.

## 8. Bounded everything

Every queue, cache, buffer and retry loop has an explicit bound and a documented
behaviour when the bound is hit (drop oldest, reject, back-pressure). Unbounded
`mpsc::unbounded_channel` is banned in production code.

## 9. Kernel features are detected, not assumed

The agent probes the kernel at startup (see
[host variance](../01-linux-foundations/05-linux-host-variance.md)), publishes a
**node feature set**, and chooses implementations accordingly. The scheduler matches
workloads requiring features to nodes that have them. There is always a documented
minimum, and below it the agent refuses to start with a clear message.

## 10. Survive restarts of anything

Every process must be restartable without disrupting workloads or losing state:

| Process restarts | What preserves continuity |
|---|---|
| server | Raft log on disk; other replicas serve meanwhile |
| agent | supervisors keep running; BPF maps pinned; checkpoint file |
| supervisor | ⚠ this is the one that can't restart transparently – keep it tiny and boring |
| container | restart policy with backoff |

## 11. Rust conventions

- **Async runtime:** tokio, multi-threaded for server and agent. Supervisor is
  *synchronous* (no tokio) to stay tiny and avoid surprises around `fork`.
- **Errors:** `thiserror` for library crates, typed error enums at module boundaries;
  `anyhow`-style catch-alls only at binary entry points.
- **Unsafe:** confined to small, audited modules (`sys` crates) wrapping syscalls;
  prefer `rustix` over raw `libc` where it covers the call. Each `unsafe` block has a
  `// SAFETY:` comment.
- **No `unwrap()`** in non-test code except where an invariant is proven and commented.
- **Tooling is Rust:** `cargo xtask` for build/test orchestration. No Python or shell
  build scripts beyond trivial wrappers.
- **Deterministic tests:** time and randomness are injected (`Clock`, `Rng` traits) so
  controllers and the scheduler can be tested with simulated time.

## 12. Compatibility as an adapter, not a constraint

Where interop with the Kubernetes ecosystem is valuable (OCI images, CSI drivers, CNI
plugins, Kubernetes manifests), we build **adapters at the edge** – an importer for
Kubernetes YAML, a CSI-compatible driver host – rather than bending the core model.

## 13. Document the trade-off when we diverge

Every divergence from Kubernetes gets an entry in the
[pain point tracker](../12-critique/06-pain-point-tracker.md) or a decision record in
the relevant doc, stating what we gain and what we give up.
