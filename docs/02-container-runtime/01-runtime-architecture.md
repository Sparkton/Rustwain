# Runtime Architecture

**K8s analogue:** CRI (Container Runtime Interface), containerd / CRI-O, containerd-shim,
runc / crun, RuntimeClass.

## How Kubernetes does it today

```
kubelet ──CRI gRPC──▶ containerd ──ttrpc──▶ containerd-shim-runc-v2 ──exec──▶ runc create/start
                         │                         │                              │
                         ├─ image service          ├─ holds stdio, reaps          └─ sets up ns/cgroup/
                         ├─ snapshotter            └─ one per pod                   seccomp, execs entrypoint,
                         └─ CNI calls (via plugin)                                   then exits
```

Per pod start this involves: kubelet → containerd (gRPC), containerd → shim (spawn +
ttrpc), shim → runc (fork/exec ×2: create, start), runc → `runc init` (re-exec of runc
inside the namespaces), CNI plugin binaries (fork/exec per plugin in the chain, each
parsing JSON from stdin), plus the pause container (a full container start of its own).

That's roughly 6–10 process executions and 3 IPC hops before your entrypoint runs.
Each layer has its own config, logs and failure modes, and the CRI abstraction is
"lowest common denominator" – features like user namespaces, image volumes, or
in-place resize require coordinated changes across kubelet, CRI spec, and every runtime.

## Rustwain's design

```
rustwain agent (one process)
 ├─ pod manager ──────────────┐
 ├─ image store               │ RuntimeBackend trait (in-process Rust calls)
 ├─ dataplane (net setup)     ▼
 └─ runtime module ──clone3+exec──▶ rustwain-supervisor (per pod)
                                     ├─ builds sandbox (ns, cgroup, mounts)
                                     └─ clone3 → container init → execve(entrypoint)
```

- **No CRI hop.** The "runtime" is a Rust module inside the agent implementing a
  `RuntimeBackend` trait. Pod manager calls it directly.
- **No runc re-exec.** The supervisor creates container processes itself (using
  library code shared with the runtime module – a `libcontainer`-style crate).
- **No pause container.** Supervisor holds namespaces (see
  [namespaces](../01-linux-foundations/01-namespaces.md)).
- **No CNI exec.** Network setup is a function call into the dataplane module, which
  configures the netns over netlink before containers start.

Per pod start: agent → spawn supervisor (1 exec), supervisor → clone container init
(no exec of a runtime binary; `execve` of the entrypoint only). **Two execs total.**

## The `RuntimeBackend` trait

Sketch (not final API):

```rust
pub trait RuntimeBackend: Send + Sync {
    /// Which isolation class this backend implements.
    fn class(&self) -> IsolationClass;

    /// Validate a pod against this backend's capabilities before admission.
    fn check(&self, pod: &ResolvedPodSpec, host: &HostFeatures) -> Result<(), Incompatibility>;

    /// Create the sandbox (namespaces/VM), return handle with netns/vsock info.
    async fn create_sandbox(&self, pod: &ResolvedPodSpec) -> Result<SandboxHandle, RuntimeError>;

    async fn start_container(&self, sb: &SandboxHandle, c: &ResolvedContainer) -> Result<ContainerHandle, RuntimeError>;
    async fn signal(&self, c: &ContainerHandle, sig: Signal) -> Result<(), RuntimeError>;
    async fn exec(&self, c: &ContainerHandle, req: ExecRequest) -> Result<ExecSession, RuntimeError>;
    async fn update_resources(&self, target: ResourceTarget, r: &Resources) -> Result<(), RuntimeError>;
    async fn stats(&self, sb: &SandboxHandle) -> Result<PodStats, RuntimeError>;
    async fn destroy_sandbox(&self, sb: SandboxHandle) -> Result<(), RuntimeError>;

    /// Re-attach after agent restart.
    async fn recover(&self) -> Result<Vec<RecoveredSandbox>, RuntimeError>;
}
```

Backends:

| Backend | Isolation class | Notes |
|---|---|---|
| `NativeBackend` | `process` | Namespaces + cgroups + seccomp + LSM, via supervisor |
| `SandboxBackend` | `sandboxed` | `[open]` gVisor-style; or native + strict seccomp/landlock "hardened process" |
| `MicroVmBackend` | `microvm` | Supervisor runs a Cloud Hypervisor process (embedded VMM later, see [KVM and microVMs](../03-virtualization/02-kvm-and-microvms.md)); guest agent starts containers |
| `VmBackend` | `vm` | Long-lived full VMs (disk images, not OCI containers) |
| `ExternalCriBackend` | any | `[open]` adapter to an existing CRI runtime for compatibility (Kata, gVisor's runsc) |

## Why not just use containerd?

| | containerd | Rustwain in-process |
|---|---|---|
| Maturity | Very high | None yet |
| Overhead | ~50–100 MB daemon + shim per pod | target ~0 extra |
| Feature velocity | Bound to CRI spec + Kubernetes release cycle | Change one codebase |
| Ecosystem (snapshotters, NRI plugins) | Large | Must build or adapt |
| Debuggability | `ctr`, `crictl` | `rwctl debug node` needed |
| Learning value | Low (we'd just configure it) | High |

The decision aligns with the project's learning goal and the efficiency goal. The
`ExternalCriBackend` keeps an escape hatch.

## Lessons from the OCI runtimes we are replacing

### runc (Go + C)
- Go's runtime can't fork safely in a multithreaded process, so runc does the namespace
  setup in a C constructor (`nsenter`/`nsexec.c`) that runs *before* the Go runtime
  starts, via re-executing itself as `runc init`. Complex, and the source of several CVEs
  (`/proc/self/exe` overwrite CVE-2019-5736; leaked fd CVE-2024-21626).
- Lesson: in Rust we control threads. The supervisor is single-threaded, so we can
  `clone3` directly with no C trampoline and no re-exec.

### crun (C)
- Faster and smaller than runc (no Go runtime), ~2× faster container creation. Shows the
  overhead runc's architecture adds.
- Supports Wasm handlers, krun (libkrun microVMs) – good ideas for pluggable executors.

### youki (Rust)
- A Rust OCI runtime, CNCF sandbox project. Its `libcontainer` crate is a reusable
  library implementing the OCI runtime spec in Rust, and `libcgroups` handles cgroups.
- Decision `[open]`: depend on `libcontainer` vs write our own. `libcontainer` is designed
  around the OCI `config.json` model and runc-compatible CLI semantics (create/start
  split, state dir) which we don't need. Proposed: **study youki closely, borrow ideas,
  write a smaller, pod-oriented crate**; revisit if ours grows large.

### Kata Containers (Rust agent + Go shim)
- Runs each pod in a lightweight VM (QEMU, Cloud Hypervisor, Firecracker, Dragonball)
  with `kata-agent` (Rust) inside. Shim v2 integrates with containerd.
- Lessons: guest agent over vsock is the right model; VM boot time and memory overhead
  are the hard parts; virtio-fs sharing image rootfs into the guest is a big perf factor.

### gVisor (Go)
- Userspace kernel ("Sentry") intercepting syscalls via ptrace/KVM/systrap platforms.
- Strong isolation, notable syscall overhead and compatibility gaps.

See [runtime pros/cons](../12-critique/02-runtimes-pros-cons.md).

## OCI compatibility

We consume **OCI images** (see [images](03-images-and-snapshotters.md)) fully. We do *not*
need to consume OCI *runtime bundles* (`config.json`) since nothing external hands us
one. However:

- Our `ResolvedContainer` will be convertible to an OCI runtime spec for debugging
  (`rwctl debug pod X --oci-spec`) and for the `ExternalCriBackend`.
- We will run the **OCI runtime-tools validation** suite against a thin CLI wrapper
  (`rustwain oci create/start/...`) as a correctness check of our container setup code –
  a free, extensive test suite.

## Resolved spec pipeline

The agent turns an API pod into something the supervisor can execute with zero further
decisions:

```
Pod (API object)
  │  + node defaults, tenant policy, security level
  ▼
AdmittedPod           ── checked against node resources, features, policy
  │  + image config (entrypoint, env, user, workdir, exposed volumes)
  │  + allocated IP, netns path, DNS config
  │  + volume host paths (after attach/mount)
  │  + uid/gid mapping range for this pod
  │  + compiled seccomp BPF, cap sets, LSM labels
  │  + cgroup paths and resource values
  ▼
ResolvedPodSpec       ── pure data, serialisable, versioned, no references to API types
  │
  ▼  written to /run/rustwain/pods/<uid>/spec.bin
supervisor
```

`ResolvedPodSpec` being pure data makes the supervisor trivially testable and makes
"why did my container get these settings?" answerable: `rwctl debug pod X --resolved`.

## Open questions

- Is a `CRI server` adapter worth building so `crictl` and kubelet could drive Rustwain's
  runtime? Interesting as a stepping stone (run Rustwain runtime under real Kubernetes to
  test it) – `[proposed]` as a milestone-1.5 testing tool, not a product feature.
- How much of youki's `libcontainer` to reuse.

## Build checklist

- [ ] `RuntimeBackend` trait and `ResolvedPodSpec` types in `rustwain-runtime-api`.
- [ ] `NativeBackend` with supervisor.
- [ ] `rustwain oci` CLI shim to run OCI runtime-tools validation tests.
- [ ] Benchmark: pod create→running vs containerd+runc on same host.
