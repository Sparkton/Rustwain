# Full VM Workloads

**K8s analogue:** KubeVirt (VirtualMachine, VirtualMachineInstance, DataVolume via CDI),
Harvester; VMware vSphere and OpenStack Nova as the incumbent VM orchestrators.

## Why VMs in a container orchestrator

- Legacy applications that can't be containerised (appliances, Windows, apps needing a
  specific kernel or kernel modules).
- Workloads needing full OS semantics (systemd, multiple services, kernel tuning).
- Consolidating VM and container infrastructure onto one control plane (one scheduler,
  one network, one storage system, one RBAC model).

KubeVirt does this by running QEMU inside a pod (`virt-launcher`) with a privileged
node daemon (`virt-handler`). It works but is layered awkwardly: the VM is a process
inside a container inside a pod, networking goes pod netns → bridge/masquerade → tap,
and live migration fights the pod model (pods are immutable and node-bound).

Rustwain can do better because VMs are a native isolation backend.

## The `VirtualMachine` kind

```yaml
kind: VirtualMachine
metadata: { name: build-server, namespace: ci }
spec:
  running: true                     # desired power state
  cpu: { cores: 4, sockets: 1, threads: 1, model: host-passthrough }
  memory: { size: 8Gi, hugepages: 2Mi?, balloon: true }
  firmware: { type: uefi, secureBoot: true, tpm: true }   # or direct-kernel
  disks:
    - name: root
      claim: build-server-root      # Claim → block-mode Disk
      bus: virtio-blk               # virtio-blk | virtio-scsi | nvme(emulated)
      boot: 1
    - name: cloudinit
      cloudInit: { userData: ..., networkData: ... }
  interfaces:
    - name: default
      network: pod                  # the cluster pod network (gets a pod IP)
      model: virtio
    - name: storage-net
      network: attachment/storage-vlan    # secondary network (multus-like)
  devices: { gpus: [ { claim: a100-1 } ] }
  evictionStrategy: LiveMigrate     # LiveMigrate | Shutdown | None
  isolation: { dedicatedCpus: true, numa: guestMappingPassthrough }
status:
  phase: Running
  node: node-7
  ip: 10.42.7.19
  guestAgent: { connected: true, os: "Ubuntu 24.04", ips: [...] }
```

- `running` (or `runStrategy`: Always, RerunOnFailure, Manual, Halted) – the VM object
  is long-lived; the **VM instance** (the running incarnation) is an internal sub-object
  bound to a node, analogous to a pod.
- Shares Claims/Disks with containers (block mode volumes).
- Same Services, network policy, RBAC.

## Implementation sketch

- Backend: Cloud Hypervisor (UEFI via OVMF or rust-hypervisor-firmware; supports
  Windows with virtio drivers) – or QEMU for maximum guest compatibility
  (`[open]`: QEMU is C and huge, but some guests need emulated devices CH doesn't have).
- The supervisor owns the VMM; no "container around the VM".
- Networking: TAP in a pod-like netns, VM gets the "pod IP" via DHCP served by the agent
  (tiny DHCP responder in the dataplane, or static via cloud-init network data). This
  keeps the VM a first-class citizen of the pod network.
- Console: serial console streamed via supervisor (`rwctl console vm/build-server`);
  VNC/SPICE optional `[later]`.
- Guest agent: optional `qemu-guest-agent`-compatible protocol over virtio-serial/vsock
  for IP reporting, freeze/thaw for consistent snapshots, graceful shutdown.

## Live migration

Moving a running VM between nodes without downtime. Required for node maintenance
(drains) without killing VMs.

### Pre-copy algorithm
1. Destination node prepares: same CPU model features (must be compatible – migration
   requires the guest CPU model to be a subset of destination host features; using
   `host-passthrough` restricts migration to identical hosts), same disks reachable
   (shared storage) or block migration, network endpoint ready.
2. Source sends all guest memory pages while the VM runs; tracks dirtied pages (KVM dirty
   log / dirty ring).
3. Iteratively resend dirty pages until the dirty set is small enough to transfer within
   the downtime target (e.g. 300 ms).
4. Pause source, send remaining dirty pages + device state + vCPU state, resume on
   destination.
5. Network cutover: the VM keeps its IP; the dataplane must move the IP's route to the
   new node (update endpoint maps, send gratuitous ARP/NDP). With our routed L3 design,
   this is a route update in BPF maps on every node (or BGP announcement).

Post-copy (start on destination immediately, fault pages from source via userfaultfd)
converges for write-heavy guests but risks VM loss if source dies mid-migration.

### Orchestration
- `Migration` object (like KubeVirt's VirtualMachineInstanceMigration): source, target
  (scheduler-chosen), phase, progress, downtime achieved.
- Triggered by drains when `evictionStrategy: LiveMigrate`.
- Bandwidth limits and concurrency limits per node (migration can saturate NICs).
- Encryption: migration stream over mTLS between agents (another exception to "agents
  don't talk directly" – dataplane-class traffic, authenticated with node certs).

## Disks and images for VMs

- **Disk import**: create a Claim pre-populated from an HTTP URL, registry
  (`containerDisk`-style: a qcow2/raw image inside an OCI artifact), or by cloning
  another Disk (KubeVirt's CDI "DataVolume"). Rustwain: `Disk.spec.source` with
  `{http, registry, clone, snapshot, blank}` handled by a volume populator controller.
- Formats: raw preferred (simplest, fastest with `io_uring`); qcow2 read support via
  conversion on import (`[open]`: write a qcow2→raw converter in Rust or use the
  `qcow2-rs` / CH's own qcow support).
- **Ephemeral VMs** from an image: CoW overlay on a shared base image (like containers).

## Container–VM parity table

| Feature | Container pod | VirtualMachine |
|---|---|---|
| Scheduling | yes | yes (same scheduler, VM requests incl. overhead) |
| Services/DNS | yes | yes |
| Network policy | yes | yes (host-side enforcement) |
| Probes | yes | yes (TCP/HTTP from host; exec via guest agent) |
| Persistent volumes | fs + block | block (fs via virtio-fs `[later]`) |
| Rolling updates | Deployment | VM replicas via `VirtualMachinePool` `[later]` |
| Live migration | no (CRIU `[open]`) | yes |
| Snapshots | volume snapshots | VM snapshot = disk snapshots + optional memory |

## Open questions

- QEMU support at all? Required for broad Windows/legacy guest compatibility. Possibly as
  an optional backend with clear "larger attack surface" labeling.
- Container live migration via CRIU: high complexity, fragile; KubeVirt-style VM migration
  is far more robust. Lean: not for containers.

## Build checklist

- [ ] `VirtualMachine` + internal `VmInstance` types and controller.
- [ ] VM backend on Cloud Hypervisor: UEFI boot, virtio-blk disks, TAP networking, serial
      console, cloud-init NoCloud disk generation.
- [ ] DHCP responder in dataplane for VM interfaces.
- [ ] Disk import populator (HTTP + registry).
- [ ] Live migration spike between two nodes with shared storage.
