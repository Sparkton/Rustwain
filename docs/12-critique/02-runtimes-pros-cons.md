# Container Runtimes and Sandboxes: Pros and Cons

## High-level runtimes (CRI implementations)

### containerd
| Pros | Cons |
|---|---|
| CNCF graduated, ubiquitous (Docker, most managed k8s) | Daemon + shim per pod; memory overhead (~30–60 MB daemon, ~5 MB+ per shim) |
| Pluggable snapshotters (overlay, stargz, nydus, soci, devmapper, erofs) | Many layers of plugins and configs (`config.toml` v2/v3 migrations) |
| Content store with leases and GC; robust | Go runtime; GC pauses (minor) |
| NRI (Node Resource Interface) plugins for runtime customisation | CRI plugin was historically a separate project merged in; complexity |
| Shim v2 API enables Kata, gVisor, Wasm shims | Shim per pod is a real per-pod cost at density |

### CRI-O
| Pros | Cons |
|---|---|
| Purpose-built for Kubernetes; version-locked to k8s minors (clear compat) | Smaller ecosystem than containerd; mostly Red Hat/OpenShift |
| conmon (C) monitors are small (~1 MB) | Uses containers/storage – lock contention issues historically |
| Simpler scope than containerd | Still one more daemon |

### Docker Engine (dockershim removed in 1.24)
| Pros | Cons |
|---|---|
| Developer UX standard | Extra layer (dockerd → containerd → runc) for k8s; removed from kubelet |

## Low-level OCI runtimes

### runc (Go + C)
| Pros | Cons |
|---|---|
| Reference implementation; most battle-tested | Re-exec via `runc init` + C nsexec trampoline; complex |
| Feature complete (cgroups v1/v2, rootless, checkpoint via CRIU) | Notable CVEs: CVE-2019-5736 (/proc/self/exe overwrite), CVE-2021-30465 (symlink race), CVE-2024-21626 (leaked fd), 2025 batch of mount/procfs races (e.g. CVE-2025-31133, -52565, -52881) |
| | Slower start than crun (Go runtime init) |

### crun (C)
| Pros | Cons |
|---|---|
| Fast (~2× runc on create), small memory | C – memory safety burden |
| Wasm handlers, libkrun integration | Smaller user base than runc |

### youki (Rust)
| Pros | Cons |
|---|---|
| Memory-safe; reusable `libcontainer` and `libcgroups` crates | Younger; fewer production deployments |
| Performance comparable to crun | OCI/CLI-shaped API, not pod-shaped |
| Great reference for Rustwain | |

## Sandboxed runtimes

### gVisor (runsc)
| Pros | Cons |
|---|---|
| Strong isolation without VMs; small host syscall surface | Syscall-heavy workloads slower; IO and networking overhead |
| Works without nested virt (systrap) | Compatibility gaps (some syscalls, /proc, io_uring) |
| Used at scale (GKE Sandbox, Cloud Run gen1) | Large Go codebase to trust |

### Kata Containers
| Pros | Cons |
|---|---|
| VM isolation with OCI/CRI compatibility | Higher memory overhead per pod; boot latency |
| Multiple hypervisors (QEMU, Cloud Hypervisor, Firecracker, Dragonball) | Complex stack (shim, agent, hypervisor, virtiofsd); debugging harder |
| Rust agent (and runtime-rs) | Nested virt requirement in clouds |
| Confidential containers (CoCo) work | Volume/devices passthrough complexity |

### Firecracker (+ firecracker-containerd)
| Pros | Cons |
|---|---|
| Minimal device model, tiny attack surface, Rust | No virtio-fs → block-device rootfs; no GPU passthrough |
| Very fast boot and snapshot restore | Not a container runtime by itself; integration work needed |
| Proven at Lambda/Fargate scale | |

### Cloud Hypervisor
| Pros | Cons |
|---|---|
| Rust (rust-vmm), modern devices (virtio-fs, VFIO, hotplug), live migration | Larger than Firecracker |
| Linux Foundation project, Intel/Microsoft/ARM involvement | Fewer turnkey integrations than QEMU |

## Image distribution tools
| Tool | Pros | Cons |
|---|---|---|
| stargz-snapshotter (eStargz) | Lazy pulling; compatible gzip | Needs image conversion; FUSE |
| SOCI | No image rebuild (separate index) | AWS-centric tooling; FUSE |
| Nydus | Fast lazy loading, in-kernel EROFS+fscache mode | Image conversion; complexity |
| Spegel | Simple P2P via containerd mirrors | containerd-specific |
| Dragonfly / Kraken | Large-scale P2P | Heavy infrastructure |

## Takeaways for Rustwain
- Collapse the daemon + shim + runtime stack into agent + tiny supervisor.
- Learn from runc CVEs: path-safe mounts (openat2/pathrs), fd hygiene, no re-exec of host
  binaries inside containers.
- Borrow youki's approach and possibly code.
- Use Cloud Hypervisor for microVMs first; keep the VMM boundary pluggable.
- Lazy pulling via in-kernel EROFS on-demand later.
