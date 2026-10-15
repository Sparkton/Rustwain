# Running on Different Linux Machines

**K8s analogue:** kubeadm preflight checks, Node Feature Discovery, the long tail of
"works on Ubuntu, breaks on RHEL" issues in every CNI/runtime project.

"Linux" is not one platform. Two machines both called Linux can differ in kernel version,
compiled-in features, init system, libc, security module, firewall backend, network
manager, filesystem, CPU architecture, page size and default sysctls. An orchestrator
that assumes one combination breaks on the others in ways that are confusing to debug.
This document lists every axis of variation we know of, how we detect it, and what we do
about it. For a per-distro lookup table of defaults and the exact probe behind each
feature, see the [kernel feature matrix](07-kernel-feature-matrix.md).

## Strategy

1. **Static binaries.** Ship `rustwain` as a statically linked binary
   (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, and `-gnu` variants if a
   dependency forces it). No dependence on host glibc version, no dynamic libraries. The
   same binary runs on Alpine and RHEL.
2. **Detect, don't assume.** A **host probe** runs at agent start (and on demand via
   `rustwain doctor`) and produces a typed `HostFeatures` struct.
3. **Choose implementations from features**, e.g. eBPF dataplane vs nftables fallback.
4. **Publish features to the API** as node attributes so the scheduler can match
   workloads (`requires: [kvm, bpf.netkit, gpu.nvidia]`).
5. **Refuse clearly below the floor.** If a hard requirement is missing, exit with a
   message naming the requirement, why it is needed, and the exact fix for the detected
   distro.
6. **Test a matrix** of real distro/kernel combos in CI (VM images, see
   [testing strategy](../13-implementation/04-testing-strategy.md)).

## Support tiers

| Tier | Meaning |
|---|---|
| **Tier 1** | Tested on every merge; bugs block release |
| **Tier 2** | Tested nightly; bugs fixed best-effort before release |
| **Tier 3** | Expected to work if requirements met; community-tested |

Proposed `[proposed]` matrix (2026):

| Distro | Versions | Kernel (default) | Tier | Notes |
|---|---|---|---|---|
| Ubuntu LTS | 24.04, 26.04 | 6.8, 6.17+ | 1 | AppArmor, systemd, netplan/systemd-networkd |
| Debian | 12, 13 | 6.1, 6.12 | 1 | AppArmor, systemd |
| Fedora | latest two | 6.x latest | 1 | SELinux enforcing, NetworkManager, firewalld |
| RHEL / Rocky / Alma | 9, 10 | 5.14 (heavily backported), 6.12 | 1 | SELinux, firewalld; RHEL 9's "5.14" has many 6.x backports – **probe features, never infer from version** |
| Amazon Linux | 2023 | 6.1 / 6.12 | 2 | SELinux permissive by default |
| Flatcar / Fedora CoreOS | current | 6.x | 2 | Immutable `/usr`, ignition; install to `/opt` |
| Alpine | 3.20+ | 6.6 LTS | 2 | musl, OpenRC (no systemd), busybox tools |
| Arch | rolling | latest | 3 | No LSM by default |
| NixOS | stable | 6.x | 3 | Non-FHS paths; needs a NixOS module |
| openSUSE / SLE | Tumbleweed, SLE 16 | 6.x | 3 | SELinux now default on new installs |
| Talos / Bottlerocket | – | – | 3 (or not supported) | They ship their own k8s-specific userland; would need dedicated integration |
| RHEL 8, Amazon Linux 2, Ubuntu 20.04 | – | 4.18 / 5.10 / 5.4 | **unsupported** | cgroup v1 default and/or kernel too old |

## Kernel floor

Decision `[proposed]`: **minimum kernel 5.15**, **recommended 6.1+**, **best 6.6+**.

What we need, and the version it arrived:

| Feature | Min kernel | Required? | Fallback |
|---|---|---|---|
| cgroup v2 with all controllers we use | 4.15–5.x | **Required** | none |
| `clone3`, `CLONE_INTO_CGROUP` | 5.3 / 5.7 | **Required** | none |
| pidfd (`pidfd_open`, `pidfd_send_signal`) | 5.3 / 5.1 | **Required** | none |
| `setns` with pidfd | 5.8 | Required | multiple `setns` calls |
| `close_range` | 5.9 | Required | iterate `/proc/self/fd` |
| `openat2` | 5.6 | **Required** | none (path safety depends on it) |
| New mount API | 5.2 | Required | — |
| Idmapped mounts (`mount_setattr`) | 5.12 (fs-dependent) | Recommended | chown copy or no-userns |
| overlayfs in userns | 5.11 | Rootless only | fuse-overlayfs |
| overlay `volatile` | 5.10 | optional | normal sync |
| `cgroup.kill` | 5.14 | Recommended | freeze + kill each pid |
| `cpu.max.burst` | 5.14 | optional | none |
| `memory.reclaim` | 5.19 | optional | no proactive reclaim |
| PSI | 4.20 | **Required** for eviction | usage-based eviction (degraded) |
| BTF (`/sys/kernel/btf/vmlinux`) | 5.4+ with `CONFIG_DEBUG_INFO_BTF=y` | Required for eBPF dataplane | nftables dataplane |
| BPF `cgroup/connect4/6`, `sendmsg`, `recvmsg`, `getpeername` hooks | 4.17 / 5.8 (getpeername) | eBPF dataplane | nftables DNAT |
| TCX (`bpf_mprog`) attach | 6.6 | optional | classic TC clsact |
| netkit devices | 6.7 | optional | veth |
| BPF ring buffer | 5.8 | eBPF dataplane | perf buffer |
| BPF timers, `bpf_loop` | 5.15 / 5.17 | optional | – |
| Landlock | 5.13 (fs), 6.7 (net) | optional | none |
| `SECCOMP_RET_USER_NOTIF` + `ADDFD` | 5.0 / 5.9 | optional (features) | feature disabled |
| io_uring | 5.1+ (usable 5.10+) | not used for workload-facing paths; may use internally | epoll |
| KVM | any | for VM classes | VM classes unavailable |
| WireGuard in-tree | 5.6 | for encryption | IPsec or none |
| nftables | 3.13 (usable 4.x+) | fallback dataplane | iptables-nft compat |
| `memfd_secret` | 5.14 | optional for secrets | regular memory |

RHEL 9's 5.14 kernel backports many 5.15–6.x features (BPF especially). This is why we
detect features by **trying** them (e.g. load a tiny BPF prog of each type, call
`clone3` with a bogus arg and check `ENOSYS` vs `EINVAL`), never by version string.

## Kernel config variance

Even at the same version, distros compile different options. The probe reads
`/proc/config.gz` or `/boot/config-$(uname -r)` when available, but **only as a hint**;
actual capability tests are authoritative.

Notable differences:
- `CONFIG_DEBUG_INFO_BTF`: on in Ubuntu, Fedora, RHEL 8.2+, Debian 11+, Arch; **off** in
  some custom/embedded kernels and older Alpine `-lts`. Without BTF, CO-RE eBPF won't
  load → we fall back or ship BTFHub-style external BTF (`[open]`).
- `CONFIG_BPF_LSM` and `lsm=` order: rarely enabled by default.
- `CONFIG_PSI_DEFAULT_DISABLED`: RHEL kernels? (some builds need `psi=1`).
- `CONFIG_USER_NS` + distro knobs (Debian `kernel.unprivileged_userns_clone`, Ubuntu
  AppArmor userns restriction, RHEL `user.max_user_namespaces` default 0 on some old
  versions).
- `CONFIG_NET_CLS_BPF`, `CONFIG_NET_SCH_INGRESS`, `CONFIG_NETKIT`, `CONFIG_VXLAN`,
  `CONFIG_GENEVE`, `CONFIG_WIREGUARD`, `CONFIG_IP_VS` – mostly modules; we may need to
  `modprobe` them. On hosts where module loading is disabled
  (`kernel.modules_disabled=1`) or the module isn't installed (minimal cloud images
  shipping `linux-image-*-virtual` without extras), features are unavailable.
- Lockdown mode (`/sys/kernel/security/lockdown` = `integrity`/`confidentiality`, common
  with Secure Boot on Ubuntu/Fedora): restricts some BPF helpers
  (`bpf_probe_read_kernel` under confidentiality), `/dev/mem`, kexec, unsigned module
  loading. Our networking BPF doesn't need restricted helpers; tracing-based features
  might.

## Init system

| Init | Distros | Implication |
|---|---|---|
| systemd | Most | Use `Delegate=yes` for cgroups; inhibitor locks for graceful shutdown; journal for agent logs; `sd_notify` readiness; systemd-resolved interaction |
| OpenRC | Alpine, Gentoo option | We must create the cgroup subtree ourselves; graceful shutdown via signal handling only; supervise agent via `supervise-daemon` |
| runit / s6 | Void, some containers | Same as OpenRC – treat as "no systemd" |
| none (agent is PID 1) | Appliance images, "Rustwain OS" `[open]` | Agent must reap and handle signals as init |

Decision: the agent works without systemd; systemd integration is an *enhancement*
detected via `$NOTIFY_SOCKET`, `/run/systemd/system` existence, and D-Bus availability.
We do not link a D-Bus library in core – for the few calls needed (inhibitor locks:
`org.freedesktop.login1.Manager.Inhibit`), use `zbus` (pure Rust) in an optional module.

### systemd pitfalls
- **cgroup ownership:** without delegation systemd may "fix" cgroups it doesn't know
  about. Always run under a unit with `Delegate=yes`.
- **`KillMode`:** agent unit must use `KillMode=process` (or put supervisors in a
  separate cgroup outside the agent's unit) so restarting the agent doesn't kill
  supervisors. Our layout puts supervisors under `rustwain.slice/...pod-*`, not under
  the agent service's cgroup, so `KillMode=control-group` on the agent only kills the
  agent.
- **Shutdown ordering:** systemd stops services in reverse dependency order; pods
  need time to drain. Use logind inhibitor `delay` locks with `InhibitDelayMaxSec` set
  ≥ our shutdown grace period (Kubernetes GracefulNodeShutdown does the same).

## libc and userland

- **glibc vs musl:** irrelevant for our static binary. Relevant for **container images**
  only in that we don't care – containers bring their own userland.
- **Missing tools:** busybox `mount`, no `iptables`, no `ip`, no `conntrack` tool, no
  `modprobe` options. Rustwain must **never shell out** for core functions; we speak
  netlink, nftables netlink, and syscalls directly.
  Exception: loading kernel modules – we can use `finit_module` directly on the `.ko`
  path we resolve under `/lib/modules/$(uname -r)` (handling `.ko.xz`/`.ko.zst`
  decompression), or exec `modprobe` if present. `[proposed]`: exec `modprobe` if
  present, else direct load, else report.
- **FHS violations:** NixOS (`/run/current-system/sw`), immutable distros (read-only
  `/usr`). Rustwain state lives in `/var/lib/rustwain` and `/run/rustwain` only, binary
  in `/usr/local/bin` or `/opt/rustwain/bin`; all paths configurable.

## LSM variance
See [security primitives](03-security-primitives.md). Probe:
- `/sys/kernel/security/lsm` lists active LSMs in order.
- SELinux mode: `/sys/fs/selinux/enforce`; presence of `container_t` type in loaded policy
  (`/sys/fs/selinux/...` class/type lookup via libselinux-free method: check
  `/sys/fs/selinux/context` validity by writing a candidate context).
- AppArmor: `/sys/kernel/security/apparmor/profiles`, `/sys/module/apparmor/parameters/enabled`.

## Firewall and packet filtering

The single most common source of "pods can't talk" bugs across distros.

| Host state | Problem | Rustwain handling |
|---|---|---|
| **firewalld** (Fedora/RHEL) | Default zone drops forwarded traffic; reloads flush rules | Add our interfaces to a `trusted` zone via firewalld D-Bus or document; eBPF dataplane largely bypasses netfilter for pod↔pod, but host ports and VXLAN UDP must be allowed |
| **ufw** (Ubuntu) | `DEFAULT_FORWARD_POLICY="DROP"` | Preflight warns; we don't rely on FORWARD chain in eBPF mode |
| iptables-legacy vs iptables-nft | Two different kernel backends; rules in one invisible to the other; mixing breaks things | We don't use iptables at all. Fallback dataplane talks **nftables netlink** directly in its own table (`table inet rustwain`) |
| Docker installed | Sets `iptables -P FORWARD DROP`, adds its own chains | Detect and warn; eBPF path unaffected |
| `br_netfilter` loaded + `net.bridge.bridge-nf-call-iptables=1` | Bridged traffic traverses iptables – needed by kube-proxy, harmful to us (extra cost, unexpected drops) | We don't use a Linux bridge in the default design; if bridge mode is used, we set these to 0 for our bridge (not possible per bridge pre-5.x? – global sysctl; `[open]`) |
| Cloud security groups / NACLs | Outside the host | Documented required ports |

Required sysctls (set by agent, per-node, and verified):
- `net.ipv4.ip_forward=1`, `net.ipv6.conf.all.forwarding=1`
- `net.ipv4.conf.all.rp_filter` – strict mode (1) breaks asymmetric routing with some
  overlay/eBPF designs; set our interfaces to 0 or 2 (loose).
- `net.ipv4.conf.<if>.forwarding`, `accept_local` for some redirect paths.
- `fs.inotify.max_user_instances` / `max_user_watches` – defaults (128 / ~8k on old
  distros) are too low for many pods using inotify; raise to 8192 / 1M.
- `kernel.pid_max` – raise to 4194304 on 64-bit for high density.
- `vm.max_map_count` – raise for workloads like Elasticsearch (per-pod not possible;
  node-level).
- `net.netfilter.nf_conntrack_max` – only matters in fallback mode.
- `kernel.keys.maxkeys`, `kernel.keys.maxbytes` – some runtimes create a keyring per
  container; we don't (we join no session keyring – avoids `keyctl` exhaustion).

## Network management daemons

| Daemon | Interference | Mitigation |
|---|---|---|
| NetworkManager | Tries to manage our veth/netkit/vxlan devices, may assign DHCP or remove routes | Drop a `/etc/NetworkManager/conf.d/rustwain.conf` with `unmanaged-devices=interface-name:rw*;interface-name:rwv*` (agent writes it at install, verifies at start) |
| systemd-networkd | Can remove "foreign" routes/rules on reload (`ManageForeignRoutes=yes` default) | Drop-in setting `ManageForeignRoutingPolicyRules=no`, `ManageForeignRoutes=no`, plus `Unmanaged=yes` link files for our devices |
| netplan | Generates networkd/NM config | Same as underlying daemon |
| ifupdown / ifupdown-ng (Debian/Alpine) | Usually ignores unknown devices | None needed |
| cloud-init | May regenerate network config on boot | Our drop-ins live in their own files |
| Wicked (SUSE) | Similar to NM | Document |

Interface naming: all Rustwain devices start with `rw` (`rwh0` host side, `rwvx0`
vxlan, `rwwg0` wireguard) so matching rules are simple.

## DNS on the host

- `systemd-resolved` puts `nameserver 127.0.0.53` in `/etc/resolv.conf`. Copying that
  into a pod (different netns) points to a nonexistent resolver – the classic CoreDNS
  "loop" bug. Rustwain reads `/run/systemd/resolve/resolv.conf` (the real upstreams)
  when resolved is active.
- See [DNS](../05-networking/05-dns-and-discovery.md).

## Time synchronisation

Raft leases, certificate validity, token expiry and lease-based failure detection all
assume clocks within a reasonable bound. Preflight checks for a running NTP client
(chronyd, systemd-timesyncd, ntpd) via `adjtimex` status (`STA_UNSYNC` flag) and warns
if unsynchronised or skew vs servers > 500 ms (agents report their time in heartbeats;
servers compare). Rustwain's **correctness never depends on synchronised wall clocks**
(leases use monotonic durations measured locally, see
[registration](../04-node-agent/05-node-registration-and-heartbeat.md)), but certificate
validity does.

## CPU architecture

| Arch | Status | Notes |
|---|---|---|
| x86_64 | Tier 1 | |
| aarch64 | Tier 1 | Graviton, Ampere, Raspberry Pi 4/5 (64-bit OS), Apple Silicon Linux VMs |
| riscv64 | Tier 3 `[open]` | Rust target exists; BPF JIT exists; KVM support is new (H-extension) |
| armv7 / 32-bit | not supported | Too many 64-bit assumptions (pid_max, BPF, memory) |
| s390x, ppc64le | not planned | |

Per-arch differences that matter:
- **Syscall numbers and seccomp:** separate compiled profiles; on x86_64 block x32
  (`__X32_SYSCALL_BIT`) and check `AUDIT_ARCH_X86_64` (deny `i386` unless enabled).
- **Page size:** x86_64 is 4 KiB; aarch64 can be 4 KiB, **16 KiB** (Asahi Linux, some
  Android-derived), or **64 KiB** (RHEL aarch64 kernels historically, some server
  distros). Affects: memory accounting granularity, hugepage sizes (64K pages → 512 MiB
  PMD hugepages), images with binaries built assuming 4K alignment (jemalloc built for 4K
  crashes on 16K kernels). Probe `sysconf(_SC_PAGESIZE)`, publish as node attribute so
  images can declare compatibility.
- **Multi-arch images:** OCI image indexes select per-platform manifests; we must pick
  `linux/arm64/v8` correctly (variant matching rules). See
  [images](../02-container-runtime/03-images-and-snapshotters.md).
- **Emulation:** `binfmt_misc` + qemu-user lets x86 images run on arm (slowly). We do not
  enable it automatically; the scheduler should place pods only on matching arches
  unless the pod opts in.
- **KVM:** x86 has `vmx`/`svm`; arm64 KVM needs EL2 boot (many SBCs and cloud
  instances don't expose nested virt). Probe `/dev/kvm` + `KVM_CHECK_EXTENSION`.
- **Heterogeneous cores** (Intel P/E cores, ARM big.LITTLE): CPU manager must know core
  types (`/sys/devices/cpu_core`, `cpu_atom`, or `cpu_capacity` on ARM) to avoid pinning
  a guaranteed pod onto efficiency cores. Published as topology info.

## Hardware and virtualization environment

- **Running inside a VM** (most cloud nodes): nested virtualization may be unavailable →
  `microvm` class unavailable unless the cloud exposes it (GCP, Azure some sizes, AWS
  `.metal` or newer nested-virt-enabled instances). Probe and publish.
- **Running inside a container** (dev/test, "Rustwain in Docker"): limited cgroup
  delegation, possibly no BPF. Supported only for development via `rustwain dev`.
- **Running on WSL2:** custom MS kernel, cgroup v2 available, BPF partially. Tier 3 dev
  only.
- **NUMA:** read `/sys/devices/system/node`. Single-socket = 1 node; publish distances.
- **Huge pages:** pre-allocated counts from `/sys/kernel/mm/hugepages/`.
- **SMT siblings:** `/sys/devices/system/cpu/cpu*/topology/thread_siblings_list` for
  full-core allocation.
- **Disks:** rotational vs SSD (`/sys/block/*/queue/rotational`), IO scheduler, for
  `io.cost` models and scheduling hints.

## Filesystem variance
See [filesystems](04-filesystems.md): xfs `ftype`, project quota mount options
(`prjquota` must be set in fstab for xfs; ext4 needs the `quota,project` features),
btrfs/zfs choices, `/var` on tmpfs (diskless nodes), `noexec` on `/var` or `/tmp`
(hardened distros – breaks anything executing from our data dir; preflight checks mount
flags of our state dirs).

## The `HostFeatures` model

```rust
pub struct HostFeatures {
    pub kernel: KernelInfo,          // release string, parsed version (hint only), arch, page_size
    pub distro: DistroInfo,          // from /etc/os-release: ID, VERSION_ID, ID_LIKE
    pub init: InitSystem,            // Systemd { version, delegation_ok } | OpenRc | Other | None
    pub cgroup: CgroupFeatures,      // controllers, psi, kill, reclaim, burst, delegated_root
    pub ns: NamespaceFeatures,       // userns allowed, idmap per fs, time ns
    pub bpf: BpfFeatures,            // btf, prog types, attach types (tcx, netkit), helpers, lockdown
    pub lsm: LsmFeatures,            // active list, selinux mode/policy ok, apparmor ok, landlock abi
    pub net: NetFeatures,            // modules (vxlan, geneve, wireguard), nftables, ipv6 enabled, mtu of uplink
    pub fs: FsFeatures,              // backing fs of state dir, d_type, project quota, overlay opts
    pub virt: VirtFeatures,          // kvm, nested, vfio/iommu groups, sev/tdx
    pub cpu: CpuTopology,            // sockets, numa, smt, core types, isolated cpus
    pub mem: MemInfo,                // total, hugepages, swap
    pub devices: Vec<DeviceInfo>,    // GPUs, NICs w/ SR-IOV, etc. (from device plugins/drivers)
    pub time: TimeSync,              // synced?, source
    pub conflicts: Vec<HostConflict>,// docker, firewalld zone, NM managing rw*, ufw forward drop, ...
}
```

Each field maps to: (a) a node attribute string for scheduling
(`kernel.bpf.netkit=true`), (b) an implementation choice in the agent, (c) a doctor
check with severity (`fatal`, `degraded`, `info`) and a remediation string specific to
the detected distro (`dnf install container-selinux` vs `apt install apparmor`).

## `rustwain doctor` output sketch

```
Host: fedora 42 (6.15.4-200.fc42.x86_64) systemd 257, SELinux enforcing
✔ cgroup v2 unified, delegated (rustwain.slice)
✔ PSI enabled
✔ BTF available; eBPF dataplane: tcx + netkit
✔ user namespaces; idmapped mounts on xfs (ftype=1)
! firewalld: default zone 'FedoraServer' will drop VXLAN (udp/8472)
    fix: firewall-cmd --permanent --add-port=8472/udp && firewall-cmd --reload
! NetworkManager may manage rw* devices
    fix: rustwain will write /etc/NetworkManager/conf.d/90-rustwain.conf (run with --fix)
✔ KVM available (vmx), nested=Y; microvm class enabled
✔ time synchronised (chronyd)
```

## Open questions

- Should we ship external BTF for kernels without `CONFIG_DEBUG_INFO_BTF`, or simply
  fall back to nftables? Fallback is simpler; BTFHub-style is more uniform.
- Do we want a minimal, purpose-built "Rustwain OS" image eventually (like Talos)? It
  would eliminate most variance but is a large project. Parked.
- RHEL's 5.14: what exact feature set is present in 9.x minor releases – build a probe
  test table from real runs.

## Build checklist

- [ ] `rustwain-probe` crate: one module per feature area, each returning typed results
      plus `DoctorCheck`s; every probe is *empirical* (try it), with timeouts.
- [ ] Distro-specific remediation strings table keyed on `ID`/`ID_LIKE`.
- [ ] `rustwain doctor [--fix]` subcommand.
- [ ] CI VM matrix for Tier 1 distros (see testing strategy).
- [ ] Node attribute publishing and scheduler matching on `requires`.
