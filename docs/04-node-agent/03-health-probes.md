# Health Probes

**K8s analogue:** liveness, readiness and startup probes (exec, httpGet, tcpSocket, grpc),
readiness gates.

## Semantics (kept from Kubernetes – they're well understood)

| Probe | Failure means | Effect |
|---|---|---|
| **Startup** | App hasn't finished starting | Other probes disabled until it succeeds; failing past threshold → restart |
| **Liveness** | App is wedged | Restart container |
| **Readiness** | App can't serve right now | Remove from service endpoints; no restart |

Parameters: `initialDelay`, `period`, `timeout`, `successThreshold`, `failureThreshold`,
`terminationGracePeriod` (per-probe override for liveness kills).

## Probe mechanisms and their cost

| Type | Kubernetes implementation | Cost | Rustwain implementation |
|---|---|---|---|
| exec | CRI ExecSync → runtime spawns process in container namespaces | **High**: fork/exec + namespace join + cgroup move each period; at 100s of pods × 1 s period this is a major CPU consumer on nodes | Supported, but lint warns; executed by supervisor with pre-built exec context. Recommend HTTP/gRPC |
| httpGet | kubelet makes HTTP request to pod IP from host netns | Low | Same; agent's shared HTTP client, connection reuse **off** (probes must test new connections, matching Kubernetes) – or configurable |
| tcpSocket | kubelet connect() | Very low | Same |
| grpc | gRPC health protocol | Low | Same (tonic health client) |
| **socket-state** (new) | – | ~Zero | `[proposed]` Check that a process in the pod has a LISTEN socket on the port via `sock_diag` netlink in the pod netns – no packets to the app at all. Good for "is it listening" readiness |
| **pressure** (new) | – | ~Zero | `[proposed]` Unready when the pod's PSI (cpu/memory) exceeds a threshold – shed load from a stalled replica before it times out |

### Probe scheduling
- A single **timer wheel** in the agent schedules all probes; a bounded worker pool
  executes them. Kubernetes uses one goroutine per probe per container.
- **Jitter**: initial offset randomised within the period to avoid thundering herds after
  node restart (all pods probing at the same instant).
- Probes on a node are rate-limited globally to protect the node under mass restarts.

### Exec probes done right
- Exec probes count against the **container's** cgroup (they run in it) – correct
  accounting, and they can't starve the node.
- Timeout is enforced via `cgroup.kill`-like semantics: exec process gets its own
  transient sub-cgroup (`c-app/probe-<n>`)? Not allowed – no internal processes rule:
  container leaf cgroup holds processes. Alternative: kill the exec'd process tree via its
  pidfd + process group. `[open]`: make container cgroups two-level (`c-app/main`,
  `c-app/exec`) so exec sessions and probes can be killed and accounted separately while
  still sharing the container limit at `c-app`. This is cleaner; costs one extra cgroup per
  container. Lean: yes.
- Kubernetes historically didn't enforce exec probe timeouts at all (fixed in 1.20 with
  `ExecProbeTimeout`) – surprising behaviour we avoid by design.

## Probes from where?

HTTP/TCP probes originate from the host network namespace to the pod IP. Consequences:
- Network policy must always allow node → pod for probes. Kubernetes NetworkPolicy
  implementations special-case host traffic inconsistently. Rustwain: probe traffic is
  marked (socket mark / cgroup of agent) and allowed explicitly by the dataplane,
  regardless of user policy; other host→pod traffic follows policy.
- For `microvm` pods, probes still go host → guest IP over the virtual NIC: works.
- Exec probes in microVMs go via guest agent over vsock.

## Readiness and endpoints flow

```
probe result change → agent pod worker → status.conditions[Ready] patch (coalesced)
  → API → endpoint controller → EndpointSet update → every agent dataplane
```

Latency matters: from "pod became unready" to "no node sends it traffic". Kubernetes:
probe period + status update (rate-limited) + endpoint controller batching (up to 1 s
by default? `--endpoint-updates-batch-period`) + kube-proxy sync period (iptables: up to
seconds at scale).

Rustwain targets < 500 ms after probe detection at 1,000 nodes:
- Status patch immediately for readiness transitions (not batched with other status).
- Endpoint controller updates with no artificial batching delay for removals (additions
  may batch briefly).
- Dataplanes apply map updates in microseconds.
- `[proposed]` **Local fast path**: the node hosting the pod immediately drops it from its
  own service maps on readiness failure (before the API round trip) – local clients stop
  hitting it instantly.

## Readiness gates
Extra conditions set by external controllers (e.g. "load balancer target registered")
that must be true for Ready. Kept as-is.

## Defaults and lint
- No default probes (same as Kubernetes) – but `rwctl lint` warns for Deployments
  behind Services without readiness probes.
- Liveness probes that check dependencies (database) are an anti-pattern (cascading
  restarts during an outage); lint can't detect that but docs call it out.
- Liveness without startup probe on slow-starting apps → restart loops; lint suggests
  startupProbe when `initialDelay > 30s`.

## Open questions

- Should failing liveness probes be rate-limited cluster-wide during incidents (a "circuit
  breaker" to avoid mass restarts when a shared dependency is down)? Interesting;
  `[open]`.

## Build checklist

- [ ] Timer-wheel probe scheduler with jitter and global concurrency limit.
- [ ] HTTP, TCP, gRPC, exec, socket-state, pressure probe types.
- [ ] Two-level container cgroup for exec sessions.
- [ ] Local fast-path removal on readiness failure.
- [ ] Benchmark: CPU cost of 500 pods × probes at 1 s period.
