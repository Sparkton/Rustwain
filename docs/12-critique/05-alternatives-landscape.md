# The Alternatives Landscape

What other orchestrators and research systems got right and wrong, and what Rustwain
borrows.

## Google Borg / Omega (papers)
- **Borg** (EuroSys 2015): centralised scheduler (with optimisations: equivalence classes,
  relaxed randomisation, score caching), Borglet node agent, Paxos-replicated
  Borgmaster, **resource reclamation** (estimating real usage, overcommitting for batch),
  priority bands (production vs non-production), allocs (reserved resource bundles), jobs
  of tasks with indices.
  - Borrow: equivalence classes in the scheduler; reclamation for batch; priority bands.
- **Omega** (EuroSys 2013): shared-state, optimistic-concurrency schedulers (multiple
  schedulers operating on a shared cell state with conflict detection).
  - Borrow (later): optimistic multi-scheduler for huge clusters.
- **"Borg, Omega, and Kubernetes"** (ACM Queue 2016): lessons – pods > raw containers,
  labels over job numbers, IP per pod (Borg's shared host ports were painful), owner
  references, declarative API.

## HashiCorp Nomad
| Pros | Cons |
|---|---|
| Single binary (server/client); simple ops | Smaller ecosystem; BUSL license since 2023 |
| Multiple task drivers (Docker, exec, Java, QEMU, raw_exec) – non-container workloads first-class | Networking/service discovery historically relied on Consul |
| Fast scheduler (optimistic concurrency, Omega-inspired) | Less extensible API (no CRD equivalent) |
| Federation across regions built in | Fewer built-in workload types |
| Borrow: single-binary model, task drivers ≈ isolation classes, optimistic scheduling | |

## Docker Swarm
| Pros | Cons |
|---|---|
| Extremely simple UX (`docker service create`) | Stagnant development; limited features |
| Built-in encrypted overlay, routing mesh, Raft in managers | Weak extensibility; limited scheduling |
| Borrow: simplicity of bootstrap; embedded Raft in managers | |

## k3s / RKE2 / k0s / MicroK8s
| Pros | Cons |
|---|---|
| Kubernetes packaged as a single binary; lower memory; SQLite via kine; batteries included (Flannel, Traefik, local-path) | Still Kubernetes underneath (same component model); k3s still ~500 MB+ RAM on server |
| Great edge/homelab story | |
| Borrow: install UX, embedded datastore, bundled defaults | |

## Talos Linux / Bottlerocket / Flatcar
| Pros | Cons |
|---|---|
| Immutable, minimal OS purpose-built for Kubernetes (Talos: API-managed, no SSH) | Less flexible; vendor-specific tooling |
| Removes host variance | Talos/Bottlerocket are OS projects in themselves |
| Borrow: the idea of a known-good host; maybe a future "Rustwain OS" image | |

## AWS ECS / Fargate, Google Cloud Run, Fly.io
- **ECS**: simpler task/service model; tightly integrated with AWS; Fargate gives
  per-task microVM isolation (Firecracker) without node management.
- **Cloud Run**: request-driven autoscaling to zero, gVisor/microVM sandboxing, Knative API
  shape.
- **Fly.io**: Firecracker microVMs from OCI images globally; their engineering blog
  documents building a custom orchestrator (flyd) after Nomad didn't fit – a cautionary
  tale about generic orchestrators vs specific needs.
- Borrow: microVM-per-workload as a normal mode; scale-to-zero; simple app-centric UX.

## Apache Mesos / Marathon / Aurora
- Two-level scheduling (resource offers to frameworks). Powerful for multi-framework
  sharing (Spark, Hadoop) but complex; lost to Kubernetes' simpler model.
- Borrow: nothing directly; lesson: a single coherent API won over flexibility.

## Slurm (HPC)
- Gang scheduling, fair share, backfill, partitions, topology-aware placement, job arrays –
  decades of batch scheduling wisdom.
- Borrow: queues/fair-share/backfill ideas for the batch scheduler.

## KubeVirt / Harvester / OpenStack Nova
- VMs on Kubernetes (KubeVirt) vs a VM-first cloud (OpenStack).
- Borrow: VM API shape (KubeVirt's VirtualMachine/VMI split, live migration objects);
  avoid VM-in-container layering.

## Firecracker-based orchestrators (e.g. Ignite – archived, Flintlock, Kata)
- Show demand for "OCI image → microVM" workflows.

## Unikraft / unikernels
- Ultra-fast boot, tiny images; compatibility costs. Interesting future isolation class
  `[open]`.

## Summary of what we borrow

| Idea | From |
|---|---|
| Declarative API, controllers, labels, pods | Kubernetes |
| Equivalence classes, reclamation, priority bands | Borg |
| Optimistic shared-state scheduling | Omega, Nomad |
| Single binary, embedded consensus | Nomad, Swarm, k3s |
| Isolation classes / task drivers | Nomad, Kata |
| Socket-LB, identity policy, netkit, EDT | Cilium |
| Node-level L4 mTLS (ztunnel) | Istio ambient |
| Queues, fair share, gang scheduling | Slurm, Kueue, Volcano |
| microVM-per-workload | Firecracker/Fargate/Fly.io |
| Immutable OS idea | Talos/Bottlerocket (future) |
