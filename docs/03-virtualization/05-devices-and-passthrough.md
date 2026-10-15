# Devices, GPUs and Passthrough

**K8s analogue:** device plugins, extended resources (`nvidia.com/gpu`), Dynamic Resource
Allocation (DRA, GA in 1.34), Container Device Interface (CDI), SR-IOV network device
plugin, NVIDIA GPU operator, KubeVirt host devices.

## The problem

Workloads need hardware that isn't CPU/memory: GPUs, FPGAs, NICs with SR-IOV virtual
functions, InfiniBand HCAs, NVMe drives, USB devices, TPMs, `/dev/kvm` itself, crypto
accelerators (QAT), TPUs/NPUs.

Questions an orchestrator must answer:
1. **Discovery**: what devices does each node have, with what attributes (model, memory,
   NUMA node, PCIe topology, MIG partitions)?
2. **Scheduling**: which node has a free device that satisfies the request, ideally
   topologically close to the CPUs/NIC the pod gets?
3. **Allocation**: which specific device instance goes to which pod, without double
   allocation?
4. **Preparation**: driver-specific setup (create a MIG slice, configure a VF's MAC/VLAN,
   bind to `vfio-pci`).
5. **Injection**: make it visible inside the container (device nodes, libraries, env
   vars, mounts) or VM (VFIO passthrough).
6. **Cleanup and health**.

## Kubernetes' two generations

### Device plugins (2017–)
- Node-local gRPC plugin advertises `vendor.com/resource: N` as an opaque counted
  resource. Scheduler just counts. Plugin's `Allocate` returns device nodes/env/mounts.
- Limitations: counts only (no attributes – "give me a GPU with ≥ 40 GB memory" is
  impossible without per-model resource names); no sharing; no cross-pod claims; poor
  topology awareness (Topology Manager helps partially); no parameters.

### DRA (Dynamic Resource Allocation)
- `ResourceSlice` (published by drivers: devices with attributes and capacity),
  `DeviceClass` (admin-defined selector), `ResourceClaim` (workload request with CEL
  selectors on attributes), scheduler allocates *structured parameters* itself.
- Supports sharing a claim among pods, attributes, consumable capacity, partitionable
  devices (MIG-like).
- Much better, but complex; driver ecosystem still catching up.

## Rustwain design: structured devices from day one

Decision `[proposed]`: **adopt DRA's structured model** (it's the right one) without the
legacy device-plugin path, and with a simpler API surface.

### Objects
```yaml
# Published by a node's device driver module (in agent or as plugin)
kind: DeviceInventory           # ≈ ResourceSlice
spec:
  node: node-7
  driver: gpu.nvidia
  devices:
    - name: gpu-0
      attributes: { model: "H100-SXM", memoryGiB: 80, numaNode: 0, pciRoot: "0000:17",
                    migCapable: true, driverVersion: "570.x" }
      capacity: { memory: 80Gi, compute: 100 }   # consumable capacity for sharing
---
kind: DeviceClass
metadata: { name: big-gpu }
spec:
  driver: gpu.nvidia
  selector: 'device.attributes.memoryGiB >= 40'
---
# In the pod spec (or a standalone DeviceClaim for sharing between pods)
spec:
  devices:
    - name: train
      class: big-gpu
      count: 2
      constraints: { sameAttribute: pciRoot }    # both GPUs under same PCIe root / NVLink domain
  containers:
    - name: trainer
      devices: [train]
```

- Expression language: a small, safe, non-Turing-complete expression language evaluated
  in Rust (CEL has a Rust implementation – `cel-interpreter` crate; or our own minimal
  one). Same language used for admission policy (see
  [admission](../07-control-plane/07-admission-and-policy.md)).
- Scheduler allocates specific device names during filtering/scoring and writes the
  allocation into the pod binding atomically with node assignment – no separate
  "reserve then allocate" dance across components.

### Device driver interface (node side)
```rust
pub trait DeviceDriver: Send + Sync {
    fn name(&self) -> &str;                                        // "gpu.nvidia"
    async fn discover(&self) -> Result<Vec<Device>, DeviceError>;  // periodic + on hotplug (udev netlink)
    async fn prepare(&self, alloc: &Allocation, pod: &PodRef) -> Result<Prepared, DeviceError>;
    async fn unprepare(&self, alloc: &Allocation, pod: &PodRef) -> Result<(), DeviceError>;
    async fn health(&self) -> Vec<DeviceHealth>;
}

pub struct Prepared {
    pub device_nodes: Vec<DeviceNode>,     // /dev/nvidia0, /dev/nvidiactl, /dev/nvidia-uvm ...
    pub mounts: Vec<Mount>,                // driver libraries (libcuda.so...) from host
    pub env: Vec<(String, String)>,        // NVIDIA_VISIBLE_DEVICES, etc.
    pub vfio: Option<VfioGroup>,           // for VM passthrough
    pub cdi_edits: Option<CdiSpecEdits>,   // when sourced from a CDI spec file
}
```

- Built-in drivers: `kvm` (expose `/dev/kvm` – for nested VMs/CI), `vfio-generic` (bind a
  PCI device to `vfio-pci` and pass to VMs), `sriov-net` (VF allocation),
  `hugepages`-style is not a device (it's a resource).
- External drivers: gRPC plugin protocol over unix socket (for vendor code we won't
  link: NVIDIA, AMD, Intel). Ideally we also **consume CDI spec files**
  (`/etc/cdi/*.yaml`, `/var/run/cdi/*.json`) which vendors already generate
  (`nvidia-ctk cdi generate`) – this gives GPU support without a vendor-specific plugin.

## GPUs in containers

What "giving a container a GPU" actually means (NVIDIA example):
- Device nodes: `/dev/nvidia0..N`, `/dev/nvidiactl`, `/dev/nvidia-uvm`,
  `/dev/nvidia-uvm-tools`, `/dev/nvidia-modeset`, `/dev/nvidia-caps/*` (MIG).
- User-space driver libraries matching the host kernel driver version (`libcuda.so`,
  `libnvidia-ml.so`, …) bind-mounted from the host, plus `ldconfig` cache update inside
  the container (the NVIDIA container toolkit runs a hook for this).
- Cgroup device eBPF allow-rules for those majors/minors.
- With userns: device nodes are owned by host root → need group mapping or idmapped
  `/dev` entries; CDI handles permissions via `additionalGids`.

Sharing:
| Mode | Isolation | Notes |
|---|---|---|
| Exclusive | Full | One pod per GPU |
| MIG (A100/H100+) | Hardware partition (memory + SMs) | Partitions created by driver in `prepare`; reconfiguration requires GPU idle |
| MPS | Shared process, limits on threads/memory | Weaker isolation |
| Time-slicing | None (same context switching) | Oversubscription only |
| vGPU (licensed) | Hypervisor-mediated | VMs |

## VFIO passthrough (VMs and microVMs)

- IOMMU must be on (`intel_iommu=on` / `amd_iommu=on` – default on AMD; Intel default on
  in recent kernels for some configs), devices grouped into **IOMMU groups** – an entire
  group must be assigned together.
- Unbind device from host driver, bind to `vfio-pci` (`driver_override` sysfs), pass
  `/dev/vfio/<group>` to the VMM.
- Probe publishes IOMMU groups and ACS (Access Control Services) capability; devices
  sharing a group can't be split between pods.
- GPU in microVM: works with Cloud Hypervisor VFIO; guest needs vendor driver; large
  BARs need appropriate guest physical address space.

## SR-IOV networking
- NIC exposes virtual functions (VFs); each VF can be moved into a pod netns (as a
  secondary or primary interface) or passed to a VM via VFIO, bypassing the host stack
  for near-line-rate networking.
- Allocation per VF with attributes (PF name, NUMA, link speed); `prepare` sets MAC,
  VLAN, spoof-check, trust, rate limits via netlink (`IFLA_VF_*`).
- Bypasses our eBPF policy enforcement (traffic doesn't traverse host TC) → must be
  restricted by policy and documented. Hardware offload of policy is `[later]`.

## Topology alignment

For high-performance pods (training, NFV), CPU, memory, GPU and NIC should be on the same
NUMA node / PCIe root. The scheduler gets topology from DeviceInventory attributes and
CPU topology; the node agent's resource manager does the final CPU pinning consistent
with the allocated devices (see
[resources, QoS, eviction](../04-node-agent/04-resources-qos-eviction.md)). Kubernetes
does this with three separate managers (CPU, memory, device) coordinated by Topology
Manager "hints" – a known source of complexity and admission failures after scheduling
(the scheduler doesn't know NUMA topology, so the kubelet rejects pods with
`TopologyAffinityError`). Rustwain: **make the scheduler topology-aware** so node-level
rejection is rare.

## Health
- Driver health checks (XID errors for NVIDIA via NVML events, link down for NICs) → mark
  device unhealthy → stop allocating, optionally evict pods using it.

## Open questions

- CEL vs a home-grown expression language for selectors.
- How far to go with dynamic MIG reconfiguration (requires draining GPU).
- Accept CDI as the primary injection format for all drivers? Strong lean yes.

## Build checklist

- [ ] DeviceInventory/DeviceClass/DeviceClaim types.
- [ ] Scheduler device allocator with attribute selectors and constraints.
- [ ] Node `DeviceDriver` trait + `kvm`, `vfio-generic`, CDI-file driver.
- [ ] gRPC external driver protocol.
- [ ] udev netlink hotplug watcher.
- [ ] Integration test with a fake device driver + scheduler allocation.
