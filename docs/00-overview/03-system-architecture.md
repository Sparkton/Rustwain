# System Architecture

**K8s analogue:** the whole of Kubernetes plus the container runtime, CNI and kube-proxy.

This document names every component, draws the boundaries between them, and walks
through what happens for the common operations. Later documents zoom into each box.

## Component map

```
                         ┌───────────────────────── rwctl / API clients ─────────────────────────┐
                         │                    (mTLS, gRPC + HTTP/JSON gateway)                    │
                         └──────────────────────────────────┬─────────────────────────────────────┘
                                                            │
 ┌──────────────────────────────── rustwain server (x3 or x5) ────────────────────────────────────┐
 │                                                                                                │
 │  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐   ┌───────────────────────────────┐    │
 │  │  API layer   │──▶│  Admission & │──▶│  Store       │◀──│ Watch hub (fan-out, caches)   │    │
 │  │ authn/authz  │   │  validation  │   │ Raft + MVCC  │   └───────────────┬───────────────┘    │
 │  │ fairness     │   │  (in-proc)   │   │ (embedded)   │                   │                    │
 │  └──────────────┘   └──────────────┘   └──────┬───────┘                   │                    │
 │                                               │ raft log replication      │ in-process watch   │
 │                                               ▼                           ▼                    │
 │                                       other servers              ┌──────────────────┐          │
 │                                                                  │ Scheduler        │          │
 │                                                                  │ Controllers      │ (leader- │
 │                                                                  │ (workloads, GC,  │  elected)│
 │                                                                  │  nodes, IPAM...) │          │
 │                                                                  └──────────────────┘          │
 └────────────────────────────────────────────────────────────────────────────────────────────────┘
                                         ▲  watch (filtered to this node)
                                         │  status updates, leases
 ┌───────────────────────────────── rustwain agent (every node) ──────────────────────────────────┐
 │                                                                                                │
 │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
 │  │ Pod manager  │  │ Image store  │  │ Dataplane    │  │ Volume mgr   │  │ DNS cache    │      │
 │  │ (lifecycle,  │  │ (pull, unpack│  │ (eBPF, IPAM, │  │ (attach,     │  │ (node-local) │      │
 │  │  probes,     │  │  snapshots,  │  │  services,   │  │  mount, fs   │  │              │      │
 │  │  eviction)   │  │  GC)         │  │  policy)     │  │  quotas)     │  │              │      │
 │  └──────┬───────┘  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘      │
 │         │ spawn / control over unix socket (or vsock for VMs)                                  │
 │         ▼                                                                                      │
 │  ┌─────────────────┐ ┌─────────────────┐ ┌──────────────────────────┐                          │
 │  │ supervisor (pod)│ │ supervisor (pod)│ │ supervisor (microVM pod) │ ...                      │
 │  │  └ containers   │ │  └ containers   │ │  └ VMM ─▶ guest agent ─▶ │                          │
 │  └─────────────────┘ └─────────────────┘ │           containers     │                          │
 │                                          └──────────────────────────┘                          │
 └────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Process inventory

| Process | Where | Count | Purpose |
|---|---|---|---|
| `rustwain server` | control-plane machines | 1 per server (3 or 5 servers) | API, store replica, watch hub, scheduler, controllers |
| `rustwain agent` | every node | 1 | Everything node-local |
| `rustwain-supervisor` | every node | 1 per pod | Holds namespaces/cgroup, reaps children, owns stdio/logs, survives agent restarts |
| VMM (Cloud Hypervisor process first; embedded rust-vmm VMM is a later learning milestone) | nodes running `microvm`/`vm` pods | 1 per VM pod | Runs the guest |
| `rustwain-guest` | inside each microVM | 1 per VM | Minimal init + container launcher inside the guest |
| `rwctl` | user machines | – | CLI |

Compare with a typical Kubernetes node: kubelet, containerd, containerd-shim-runc-v2
per pod, runc (transient), pause container per pod, kube-proxy, CNI daemon (calico-node,
cilium-agent…), CoreDNS/NodeLocal DNSCache, CSI node plugins, log agent.

## Communication paths

| From → To | Transport | Auth | Notes |
|---|---|---|---|
| Client → API | gRPC over HTTP/2 + JSON gateway | mTLS or bearer token | gRPC is the canonical API; JSON gateway for humans/scripts |
| Server ↔ Server (Raft) | gRPC streams | mTLS with server certs | Separate port from client API |
| Agent → API | gRPC streams | mTLS with node cert | Agents watch their own pods plus the node bundle (objects those pods reference), and cluster-wide dataplane state (services, endpoint sets, identities, policy). The latter is O(cluster) per node by nature, so it is sliced and delta-encoded |
| Agent → Supervisor | Unix socket, length-prefixed protobuf/`postcard` | Filesystem permissions + `SO_PEERCRED` | Local only |
| Agent → Guest agent | vsock | VM is only reachable from its own supervisor | |
| Pod → Pod | Kernel networking + eBPF | Network policy by identity | Optional WireGuard between nodes |
| Pod → API | Service VIP for the API | Workload identity token or mTLS | |

Decision `[proposed]`: **gRPC (tonic) as canonical wire protocol, protobuf schemas
generated from Rust types.**
Alternatives: REST/JSON only (Kubernetes' choice, simpler tooling, slower and
schema-loose); Cap'n Proto (zero-copy, smaller ecosystem); custom binary (fastest, worst
tooling).
Why: strong streaming support for watch, binary efficiency, HTTP/2 multiplexing, and the
JSON gateway preserves curl-ability.

## Walkthrough 1: `rwctl apply -f web.yaml` (a Deployment)

1. `rwctl` parses the file, converts it to the typed object, and issues a **server-side
   apply** request over gRPC with its field-manager name.
2. **API layer** on any server:
   - Authenticates (client cert or token).
   - Classifies the request into a fairness bucket and admits it or queues it.
   - Authorizes (RBAC check: can this identity `apply` `deployments` in this namespace?).
3. **Admission**: defaulting, then validation (built-in Rust validators, then any
   in-process policy rules), then mutation policy, then final validation.
4. **Store**: if this server is not the Raft leader, the write is forwarded to the leader.
   The leader appends to the Raft log, replicates to a quorum, applies to the MVCC state
   machine, and assigns a **revision** (the Raft index). The response returns the new
   object with its revision.
5. **Watch hub** on every server sees the committed write (it is applied locally on each
   replica) and fans it out to subscribers.
6. **Deployment controller** (leader-elected, running on one server) receives the event,
   computes the desired ReplicaSet-equivalent, and writes pods (or a replica set object,
   see [workload API](../08-workloads/01-workload-api.md)).
7. **Scheduler** receives the unscheduled pods, filters and scores nodes using its
   in-memory cluster snapshot, and writes a **binding** (sets `spec.node`).
8. The **agent** on the chosen node is watching "pods where node == me" and receives the
   pod.
9. Agent **admits** the pod locally (resources, features, security), allocates an IP,
   pulls images, prepares volumes, spawns a **supervisor**, which builds the sandbox and
   starts the containers.
10. Agent reports `status` (IP, container states, conditions) back via the API.
11. **Endpoint controller** sees the pod become ready and updates the Service's endpoint
    set; every agent's **dataplane** receives the update and atomically updates its eBPF
    service maps.

Target end-to-end for steps 1–10 with a warm image and a single replica: < 1 s.

## Walkthrough 2: a packet from pod A (node 1) to Service S

1. Pod A calls `connect()` to S's virtual IP.
2. The **cgroup/connect4** eBPF hook (socket-level load balancing) in the agent's
   dataplane rewrites the destination to a chosen backend pod B's IP *before the packet
   exists*. No NAT, no conntrack entry for the service translation.
3. The packet leaves A's interface; a TC eBPF program on the host side checks
   network policy (A's identity → B's identity), then routes:
   - same node: redirect directly to B's interface (`bpf_redirect_peer`);
   - other node: native routing, or encapsulate (VXLAN/Geneve) or encrypt (WireGuard).
4. On node 2, ingress TC program re-checks policy and delivers to B.
5. Replies follow the reverse path; `getpeername()` on A still shows S's VIP because the
   socket-LB hook also handles reverse translation for the socket.

See [eBPF dataplane](../05-networking/03-ebpf-dataplane.md).

## Walkthrough 3: node failure

1. Node 3 stops renewing its **lease** (default every 5 s, TTL 20 s; tunable).
2. The **node lifecycle task** marks the node `Unknown` and taints it `unreachable`. It runs
   on the Raft leader, because the leader-local lease table lives there (see
   [state store](../07-control-plane/02-state-store.md#3-leader-failover-with-leader-local-leases)).
3. After the toleration window (per-pod, default 60 s for stateless), pods are marked
   for deletion and their owners create replacements, which the scheduler places
   elsewhere.
4. For pods with **exclusive** volumes (claims with `ReadWriteOnce` semantics), the
   storage controller does not reattach the disk elsewhere until it has proof the old
   node is fenced (lease expired *and* fencing action succeeded, or an operator
   confirms). This avoids the classic split-brain double-mount.
5. If node 3 comes back, its agent learns from the API that its pods were deleted and
   tears them down; no "zombie" pods keep serving.

See [HA and failure modes](../10-cluster-operations/03-ha-and-failure-modes.md).

## Walkthrough 4: agent restart (upgrade)

1. Agent process exits (upgrade, crash).
2. Supervisors keep running; containers are unaffected; the eBPF programs and maps are
   **pinned** in bpffs so networking continues.
3. New agent starts, reads its on-disk checkpoint, reconnects to every supervisor socket
   (`/run/rustwain/pods/*/ctl.sock`), re-adopts pinned BPF maps, re-syncs with the API, and
   reconciles any differences.
4. Zero workload disruption is a hard requirement for agent restarts.

## Layering rules

1. **The store knows nothing about workloads.** It stores bytes keyed by path with
   revisions. Typing lives in the API layer.
2. **Controllers never talk to agents directly.** All coordination goes through the API
   (level-triggered, observable, restartable).
3. **Agents never talk to each other for control.** Only dataplane traffic flows between
   nodes. (Exception `[open]`: peer-to-peer image distribution.)
4. **The supervisor knows nothing about the cluster.** It takes a fully resolved local
   spec and executes it.
5. **Every component can be run alone for testing**: store as a library, API against an
   in-memory store, agent in `--standalone` mode reading pod specs from a directory.

## What runs where, by deployment size

| Shape | Servers | Notes |
|---|---|---|
| Laptop / single node | 1 server+agent in one process | `rustwain dev` – no Raft quorum, still uses the Raft log for uniformity |
| Small (≤ 50 nodes) | 3 servers, also agents | Typical homelab/edge |
| Medium (≤ 1,000 nodes) | 3 dedicated servers | |
| Large (≤ 5,000+ nodes) | 5 dedicated servers, optional read-only watch replicas | Watch replicas = learners that serve reads/watches but don't vote |

## Related

- [Design principles](04-design-principles.md)
- [Node agent architecture](../04-node-agent/01-node-agent-architecture.md)
- [API server](../07-control-plane/01-api-server.md)
- [State store](../07-control-plane/02-state-store.md)

## Open questions

- Should the scheduler and controllers run in the server process or as separate
  processes for blast-radius isolation? Current lean: in-process, with an option to run
  them standalone (`rustwain controllers --kubeconfig…`) for development.
- Do agents connect to every server, or to one with failover? Lean: one, chosen by
  hashing the node name, with client-side failover.

## Build checklist

- [ ] Define the four binaries' CLI entry points and config files.
- [ ] Define the agent↔supervisor protocol (versioned).
- [ ] Prototype walkthrough 1 end-to-end with in-memory store and a fake agent.
- [ ] Write the "agent restart without disruption" integration test before writing the agent.
