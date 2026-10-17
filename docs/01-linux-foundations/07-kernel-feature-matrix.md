# Kernel and Host Feature Matrix

**K8s analogue:** none in-tree. The closest equivalents are kubeadm preflight checks,
Node Feature Discovery labels, Cilium's "system requirements" page and the long tail of
distro-specific issues in every CNI and runtime project.

[Linux host variance](05-linux-host-variance.md) lists every *axis* of variation and the
`HostFeatures` model. This document is the companion **lookup table**: for each tier-1
and tier-2 distro, what the default install gives us on each axis, and the exact probe
the agent runs to find out on a real machine.

> **Trust the probe, not the table.** The table describes *default installs* of each
> distro as far as we know them in late 2026. Cloud images, hardened builds, custom
> kernels and point releases differ. Anything we have not confirmed on a real machine is
> marked **[verify]**. The table is a planning aid and a source of remediation strings;
> the agent never reads it to decide anything. Deciding is done by the probes in
> [the probe catalogue](#probe-catalogue).

## Per-distro defaults

| Distro | Default kernel | cgroup mode | Active LSM (major) | BTF (`/sys/kernel/btf/vmlinux`) | PSI default | userns restrictions | Firewall daemon | Network manager |
|---|---|---|---|---|---|---|---|---|
| **Ubuntu 24.04 LTS** | 6.8 (GA); HWE kernels later (6.11 → 6.14 → 6.17 [verify]) | v2 unified | AppArmor (+ Landlock, Yama, lockdown) | Yes | Enabled | **Restricted**: `kernel.apparmor_restrict_unprivileged_userns=1`. Unprivileged, unconfined processes cannot get capabilities inside a new userns unless an AppArmor profile allows `userns` | `ufw` installed, **inactive** by default | Server: netplan → systemd-networkd. Desktop: NetworkManager. systemd-resolved stub on 127.0.0.53 |
| **Ubuntu 26.04 LTS** | 7.0 (final, not an rc; confirmed by Canonical's kernel team) | v2 unified only (ships systemd 259; systemd 258 removed cgroup v1 support [verify]) | AppArmor (+ Landlock, Yama, lockdown) | Yes | Enabled [verify] | Same AppArmor userns restriction as 24.04 [verify] | `ufw`, inactive [verify] | As 24.04 |
| **Debian 12** (bookworm) | 6.1 LTS | v2 unified | AppArmor (enabled by default since Debian 10) | Yes | **Compiled in but disabled by default** (`CONFIG_PSI_DEFAULT_DISABLED=y`; needs `psi=1`) [verify] | Unprivileged userns allowed (`kernel.unprivileged_userns_clone=1`, Debian-specific sysctl) | None (nftables package present, service disabled) | ifupdown (`/etc/network/interfaces`); NetworkManager on desktop installs |
| **Debian 13** (trixie) | 6.12 LTS | v2 unified | AppArmor | Yes | Disabled by default, as Debian 12 [verify] | Allowed, as Debian 12 [verify] | None | ifupdown (server); NetworkManager (desktop); `/tmp` is tmpfs by default |
| **Fedora** (current two releases) | Latest stable upstream, rebased during the release's life (6.x/7.x) | v2 unified | **SELinux enforcing** (targeted) | Yes | Enabled (systemd-oomd depends on it and is on by default) | Allowed | **firewalld** (nftables backend) | NetworkManager; systemd-resolved |
| **RHEL 9** (and Rocky/Alma 9) | 5.14.0-*N*.el9 with heavy backports | v2 unified (v1 still bootable via `systemd.unified_cgroup_hierarchy=0`) | **SELinux enforcing** | Yes | **Disabled by default; needs `psi=1`** on the kernel command line [verify] | Allowed (`user.max_user_namespaces` non-zero) | **firewalld** (nftables backend) | NetworkManager (keyfile format; ifcfg deprecated) |
| **RHEL 10** (and clones) | 6.12.0-*N*.el10 | v2 only (v1 removed [verify]) | **SELinux enforcing** | Yes | Disabled by default, as RHEL 9 [verify] | Allowed | **firewalld** | NetworkManager |
| **Amazon Linux 2023** | 6.1 (default), 6.12 selectable [verify]; newer kernel streams may be added | v2 unified | SELinux, **permissive** | Yes | Enabled [verify] | Allowed | None by default | systemd-networkd (+ `amazon-ec2-net-utils` managing ENIs); systemd-resolved |
| **Alpine** 3.21+ | `linux-lts` 6.12 / `linux-virt` 6.12 [verify] | OpenRC `rc_cgroup_mode` in `/etc/rc.conf`; default `unified` since OpenRC 0.51 (some first-boot provisioning still forces `hybrid` – probe) | None of SELinux/AppArmor by default (Landlock/Yama compiled in [verify]) | Yes on `linux-lts` [verify]; check `linux-virt` separately | Enabled [verify] | Allowed | None (iptables/nftables packages optional, `awall` optional) | ifupdown-ng (busybox `ifupdown` on minimal); no systemd, no resolved |
| **Flatcar** (stable channel) | 6.6 LTS, moving to 6.12 LTS [verify] | v2 unified | SELinux present, **permissive** by default [verify] | Yes | Enabled [verify] | Allowed | None | systemd-networkd; systemd-resolved. `/usr` read-only (install to `/opt`) |

### What each row means for Rustwain

| Distro | Doctor findings on a default install | Agent behaviour |
|---|---|---|
| Ubuntu 24.04 / 26.04 | `info`: AppArmor userns restriction active | Root agent unaffected (it holds `CAP_SYS_ADMIN` in the init userns). **Rootless mode** needs our AppArmor profile with `userns,` installed for the rootless agent binary – `rustwain doctor --fix` writes `/etc/apparmor.d/rustwain-rootless` |
| Ubuntu (desktop) | `warn`: NetworkManager may manage `rw*` | Write NM drop-in (see [host variance](05-linux-host-variance.md)) |
| Debian 12 / 13 | `degraded`: PSI disabled | Eviction falls back to usage polling at 1 s; remediation: add `psi=1` to `GRUB_CMDLINE_LINUX` + `update-grub` + reboot |
| Fedora | `warn`: firewalld zone will drop overlay UDP and NodePorts | Add `rw*` interfaces to a `rustwain` zone via firewalld D-Bus (`--fix`), or print `firewall-cmd` commands |
| RHEL 9 / 10 | `degraded`: PSI disabled; `warn`: firewalld | `grubby --update-kernel=ALL --args=psi=1` + reboot; firewalld as Fedora; `container-selinux` policy package required for `container_t` |
| RHEL 9 specifically | `info`: kernel 5.14 – BPF features probed individually | Usually tcx/netkit absent → clsact + veth paths [verify per minor release] |
| AL2023 | `info`: SELinux permissive | No change; label as on enforcing hosts so switching to enforcing works |
| Alpine | `info`: no systemd | Agent creates and owns its cgroup subtree directly; OpenRC service file; `degraded` if `rc_cgroup_mode=hybrid` (refuse: cgroup v2 unified is required) |
| Flatcar | `info`: read-only `/usr` | Install under `/opt/rustwain`; systemd units via Ignition/Butane |

## Probe catalogue

Every probe is **empirical** – it exercises the feature – and has a timeout (default
2 s). Version strings and config files are recorded as hints only. Each probe returns a
typed result into `HostFeatures` (see [host variance](05-linux-host-variance.md#the-hostfeatures-model)).

| Feature | Exact probe (in order) | Result type | Notes |
|---|---|---|---|
| Kernel release | `uname(2)` → `release`, `machine`; `getauxval(AT_PAGESZ)` for page size | `KernelInfo` | Parsed version is a **hint only** (RHEL backports) |
| Distro | Parse `/etc/os-release` (`ID`, `ID_LIKE`, `VERSION_ID`), fall back to `/usr/lib/os-release` | `DistroInfo` | Used only to pick remediation strings |
| cgroup mode | `statfs("/sys/fs/cgroup")`: `f_type == CGROUP2_SUPER_MAGIC (0x63677270)` → **unified**. `TMPFS_MAGIC` and `statfs("/sys/fs/cgroup/unified")` is cgroup2 → **hybrid**. Otherwise **legacy** | `CgroupMode` | Hybrid/legacy is `fatal` |
| cgroup controllers + delegation | Read `/sys/fs/cgroup/cgroup.controllers`; create `rustwain.slice/probe-<rand>`; write `+cpu +memory +io +pids +cpuset` to the parent's `cgroup.subtree_control`; check each `EBUSY`/`ENOENT`/`EACCES`; `rmdir` | `CgroupFeatures.controllers` | Detects missing `Delegate=yes` under systemd |
| `cgroup.kill`, `memory.reclaim`, `cpu.max.burst` | `access()` on the file inside the probe cgroup | bools | Existence = support for these interface files |
| PSI | 1) `open("/proc/pressure/memory", O_RDONLY)` and `read` – `ENOENT` or `EOPNOTSUPP` ⇒ disabled. 2) Inside the probe cgroup, open `memory.pressure` `O_RDWR`, write trigger `"some 150000 1000000"`, `poll()` for `POLLPRI` registration success (no wait for an event) | `PsiSupport { system, cgroup, triggers }` | Both read and trigger paths matter – eviction uses triggers. Hint: `/proc/cmdline` contains `psi=1` / `psi=0` |
| Active LSMs | Read `/sys/kernel/security/lsm` (mount `securityfs` at `/sys/kernel/security` if absent) | `Vec<Lsm>` in order | Comma-separated, e.g. `lockdown,capability,landlock,yama,apparmor` |
| SELinux mode | Read `/sys/fs/selinux/enforce` (`1` enforcing, `0` permissive); absent ⇒ disabled. Policy check: write candidate context `system_u:system_r:container_t:s0` to `/sys/fs/selinux/context` – `EINVAL` ⇒ `container_t` not in policy | `SelinuxState { mode, container_policy }` | No libselinux dependency |
| AppArmor | Read `/sys/module/apparmor/parameters/enabled` (`Y`); check `/sys/kernel/security/apparmor/profiles` readable | `AppArmorState` | |
| Landlock ABI | `landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION)` → ABI number, or `ENOSYS`/`EOPNOTSUPP` | `Option<u32>` | ABI ≥ 4 ⇒ TCP bind/connect rules |
| Lockdown | Read `/sys/kernel/security/lockdown` – the file reads like `none [integrity] confidentiality`, and the bracketed word is the active mode | `Lockdown` | `confidentiality` restricts some BPF helpers |
| BTF | `open("/sys/kernel/btf/vmlinux")`, read the 24-byte header, check magic `0xEB9F` and version 1; then `BPF_BTF_LOAD` a minimal BTF blob to confirm the kernel accepts program BTF | `BtfSupport { vmlinux, prog_btf }` | Gates only optional features (kfuncs, fentry, BPF-LSM). The core dataplane needs no kernel BTF |
| Dataplane verifier acceptance | **Authoritative:** load every shipped dataplane program (no attach) and run the TC/XDP ones once with `BPF_PROG_TEST_RUN` on a crafted packet | `Vec<(ProgramId, LoadResult)>` | Proves the verifier on *this* kernel accepts our Rust-emitted BPF. Verifier behaviour differs by kernel |
| BPF program/attach types | For each needed type (`cgroup_sock_addr` connect4/6, sendmsg, recvmsg, getpeername; `sched_cls`; `xdp`; `sk_lookup`): `BPF_PROG_LOAD` of a 2-instruction program (`r0 = 1; exit`) with the right `expected_attach_type` → ok / `EINVAL` | `BpfFeatures.prog_types` | Never infer from version |
| TCX | Create a dummy device in a throwaway netns; `BPF_LINK_CREATE` with `BPF_TCX_INGRESS` → ok / `EINVAL` | bool | 6.6+ upstream |
| netkit | In the throwaway netns: `RTM_NEWLINK` with `IFLA_INFO_KIND="netkit"` → ok / `EOPNOTSUPP` | bool | 6.7+ upstream; also needs `CONFIG_NETKIT` |
| BPF ring buffer | `BPF_MAP_CREATE` with `BPF_MAP_TYPE_RINGBUF`, 4 KiB | bool | |
| User namespaces (root) | `fork()`; child `unshare(CLONE_NEWUSER)`; write `uid_map`/`gid_map` with a 65536 range; `mount` a tmpfs inside a new mount ns; exit code reports the first failing step | `UsernsSupport.root` | |
| User namespaces (unprivileged / rootless) | Same as above in a child that first drops to the configured rootless UID (`setresuid`) | `UsernsSupport.unprivileged` | Hints: `/proc/sys/kernel/apparmor_restrict_unprivileged_userns`, `/proc/sys/kernel/unprivileged_userns_clone` (Debian), `/proc/sys/user/max_user_namespaces` |
| Idmapped mounts per fs | In the probe userns: `open_tree(state_dir, OPEN_TREE_CLONE)` + `mount_setattr(MOUNT_ATTR_IDMAP)` → ok / `EINVAL` | `HashMap<FsType, bool>` | Per backing filesystem of the state dir and each volume root |
| Firewall daemon | 1) System D-Bus (via `zbus`, pure Rust): `GetNameOwner("org.fedoraproject.FirewallD1")`; if owned, query default zone and whether forwarding/our ports are allowed. 2) `ufw`: parse `/etc/ufw/ufw.conf` `ENABLED=yes` **and** confirm via nftables netlink that `ufw-*` chains exist. 3) Enumerate nftables tables over netlink (`NFT_MSG_GETTABLE`) and report foreign ones (`inet firewalld`, `ip filter` with `DOCKER*` chains, `ip nat` with `KUBE-*`) | `Vec<HostConflict>` | No shelling out to `firewall-cmd`/`nft` |
| Network manager | D-Bus `GetNameOwner` for `org.freedesktop.NetworkManager`, `org.freedesktop.network1` (systemd-networkd); file hints: `/etc/netplan/*.yaml`, `/etc/network/interfaces`, `/run/network/ifstate`. Then **verify our drop-ins took effect**: create `rwprobe0` (dummy) and check after 2 s that it has no addresses/routes we didn't add | `NetManager` + `Vec<HostConflict>` | The post-check catches "drop-in present but ignored" |
| DNS stub | Read `/etc/resolv.conf`; if any `nameserver` is in `127.0.0.0/8` or `::1`, read `/run/systemd/resolve/resolv.conf` for real upstreams | `HostDns` | Prevents the copied-127.0.0.53 loop |
| Init system | `/proc/1/comm` (`systemd`, `openrc-init`, `init`, …) + for systemd, D-Bus `org.freedesktop.systemd1` `Version` property; confirm `Delegate=yes` by checking our unit's cgroup `cgroup.subtree_control` is writable | `InitSystem` | |
| Time sync | `adjtimex(2)` with `modes=0`: `STA_UNSYNC` clear ⇒ synced; `maxerror` reported | `TimeSync` | Matters for opt-in lease reads and cert validity |

## Keeping the table honest

- **Generated, not hand-maintained** `[proposed]`: CI's distro VM matrix
  ([testing strategy](../13-implementation/04-testing-strategy.md)) runs
  `rustwain doctor --json` on a stock image of each row. `cargo xtask host-matrix`
  collects the JSON and regenerates the *Per-distro defaults* table between marker
  comments. A **[verify]** cell disappears the first time CI observes the value.
- Cells that change within a release (Ubuntu HWE kernels, Fedora rebases, AL2023 kernel
  streams) record the **probed kernel** alongside the date of the run.
- When a probe result disagrees with this table on a user's machine, the doctor output
  says so ("expected PSI disabled on debian 12; found enabled") – useful signal, never an
  error.

## Open questions

- Should `psi=1` on Debian/RHEL be a hard requirement (refuse to start) rather than
  `degraded`? Usage-polling eviction is the Kubernetes status quo, so `degraded` keeps
  parity; but the graduated, PSI-driven design in
  [eviction](../04-node-agent/04-resources-qos-eviction.md) is a headline feature. Lean:
  `degraded` with a loud doctor warning. Tracked as `PP-R1`.
- Ship our own AppArmor profile for rootless mode on Ubuntu, or require the user to opt
  in? Lean: ship, install only on `rustwain doctor --fix`.
- Alpine `linux-virt` BTF status – if absent, nftables fallback on the most common Alpine
  cloud kernel.

## Build checklist

- [ ] Every probe in the catalogue implemented in `rustwain-probe` with a unit-testable
      parser and an integration test in the distro VM matrix.
- [ ] `rustwain doctor --json` schema (stable, versioned).
- [ ] `cargo xtask host-matrix` to regenerate the defaults table from CI results.
- [ ] Remediation strings for each `degraded`/`warn` finding above, keyed by `ID`/`ID_LIKE`.
