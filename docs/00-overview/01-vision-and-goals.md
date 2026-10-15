# Vision and Goals

## One sentence

Rustwain is a cluster orchestrator, written entirely in Rust, that takes a set of Linux
machines and runs containers, sandboxes and virtual machines across them with the
smallest possible overhead, the fewest moving parts, and failure behaviour you can
reason about.

## Why build this at all

Kubernetes won. It is the lingua franca of running software on fleets of machines,
and it deserves much of that success: the declarative API, the reconciliation model
and the extensibility story are genuinely good ideas. But a decade of production use
has also exposed costs that are structural rather than incidental:

- **Too many processes for one job.** A node runs kubelet, a CRI daemon (containerd or
  CRI-O), a per-container shim, an OCI runtime binary, kube-proxy, a CNI daemon, often a
  DNS cache and a log shipper. Each has its own config, its own failure modes and its
  own memory footprint. The boundaries were drawn to let separate organisations own
  separate pieces, not because the problem demanded them.
- **Data planes built on the wrong primitive.** kube-proxy's iptables mode is O(n) in
  services for rule updates and connection setup. IPVS helped; eBPF dataplanes such as
  Cilium fixed it, but as a bolt-on with its own control plane.
- **etcd as a black box.** The only state store is an external Raft database with a
  hard size ceiling, a request size limit, and compaction/defragmentation chores that
  operators must babysit.
- **Garbage-collected control plane.** Go is productive, but API server and controller
  memory scales with cluster size in ways that are hard to bound, and tail latencies
  include GC pauses.
- **Isolation is an afterthought.** Running untrusted code requires bolting on gVisor or
  Kata through RuntimeClass; user namespaces only became usable for pods recently.
- **Operational surface.** Certificates, upgrades with strict version skew rules,
  deprecation churn, YAML sprawl, webhooks that can take the whole API down.

Rustwain is a learning project first. The goal is to understand each of these
problems deeply enough to design something better, then build it. The constraint of
"Rust only, from kernel interface to CLI" forces us to understand every layer instead of
delegating it to an existing binary.

## Goals

### G1 – Cover the same problem space
A user should be able to do what they do with Kubernetes:

- Describe desired state declaratively and have the system converge to it.
- Run long-lived services, batch jobs, per-node daemons and stateful sets.
- Give every workload an IP, a DNS name, and a stable virtual address for groups.
- Attach persistent storage that follows workloads across machines.
- Enforce resource limits, quotas, network policy and identity.
- Roll out changes safely, scale up and down, survive machine failures.
- Extend the system with new resource types and controllers.

### G2 – Unify isolation levels
Containers, sandboxed containers, microVMs and full VMs are all first-class
**isolation classes** of the same workload object. Switching a workload from a
namespace-based container to a microVM is a one-field change, not a different product.

### G3 – One binary per role, few roles
- `rustwain server` – API, state store, scheduler, controllers (all in-process, all
  optional per node).
- `rustwain agent` – node agent, runtime, networking dataplane, DNS cache, volume
  manager (all in-process, with one tiny per-workload supervisor).
- `rwctl` – the CLI.

### G4 – Efficient by construction
We publish budgets (see [performance budgets](../11-efficiency/01-performance-budgets.md))
and test against them in CI. Headline targets:

| Metric | Kubernetes typical | Rustwain target |
|---|---|---|
| Node agent idle RSS (all node components) | 150–400 MB | < 40 MB |
| Per-container supervisor RSS | ~5–10 MB (containerd-shim) | < 1 MB |
| Pod start, warm image, p50 | 1–2 s | < 300 ms |
| Service rule update, 10k services | seconds (iptables) | < 10 ms, O(1) |
| Pods per node | 110 default | 500+ supported, limited by hardware |
| Control plane memory, 5k nodes / 150k pods | tens of GB | target < 8 GB |

These targets are aspirational and will be refined as we measure.

### G5 – Run on ordinary Linux
Any mainstream distribution with a reasonably modern kernel, on x86_64 and aarch64,
should be able to join. See [Linux host variance](../01-linux-foundations/05-linux-host-variance.md)
for the exact support matrix and how we detect and adapt to differences.

### G6 – Visible trade-offs
Every place where we diverge from Kubernetes, or where Kubernetes has a known problem,
is tracked in the [pain point tracker](../12-critique/06-pain-point-tracker.md) with an
explicit status. The project should make it easy to see what is better, what is worse,
and what is not yet done.

## Non-goals (for now)

- **Wire compatibility with the Kubernetes API.** We may add a compatibility gateway
  later (see [API server](../07-control-plane/01-api-server.md)), but we will not
  constrain the core model to match Kubernetes' JSON shapes.
- **Running on non-Linux hosts.** Windows and macOS nodes are out of scope. macOS may
  be a client (CLI) platform only.
- **Frontend / dashboard / WASM.** The CLI and API are the interface.
- **Observability stack.** Deferred. We design hooks (structured events, metrics
  endpoints) but do not build a monitoring product. See
  [observability notes](../10-cluster-operations/06-observability.md).
- **Being a PaaS.** No built-in buildpacks, CI or app catalog.
- **Cloud-provider integrations in core.** Load balancers and disks from cloud APIs are
  plugins, never linked into the core binaries.

## Success criteria by phase

1. **Single node:** `rustwain agent --standalone` can run a pod spec (multiple
   containers, shared network, volumes, probes, restart policy) on Ubuntu, Fedora,
   Debian and Alpine hosts, with cgroup v2 and user namespaces.
2. **Small cluster:** three servers plus N agents, Raft-replicated state, scheduler,
   Deployment/Service equivalents, eBPF service load balancing, cross-node pod
   networking.
3. **Hard isolation:** the same pod spec runs as a microVM with a flag change.
4. **Production-shaped:** rolling upgrades of Rustwain itself, chaos tests passing,
   budgets enforced in CI, 1,000-node simulated scale test.

## Who this is for

A single engineer (and agents) learning by building. That shapes the design: favour
fewer, deeper components over many shallow ones; favour designs that can be tested
on one laptop with VMs; favour clarity over cleverness until profiling says otherwise.
