# Bootstrap and Join

**K8s analogue:** kubeadm init/join (preflight, certs, kubeconfigs, static pod manifests
for control plane, bootstrap tokens, cluster-info ConfigMap), k3s/RKE2 single binary
install, Talos machine config, Cluster API.

## Goals
- **One command** to start a cluster, **one command** to join a node.
- Secure by default (no plaintext tokens in long-lived places; CA pinning on join).
- Same binary for every role; no external dependencies (no etcd, no container runtime, no
  CNI binaries to install).
- Works offline/air-gapped (with a local registry for workload images).

## Install

```bash
curl -fsSLO https://.../rustwain-<ver>-<arch>.tar.zst   # plus signature verification
rustwain install            # copies binary, writes systemd unit (or OpenRC script), runs doctor
```

`rustwain install` writes:
- `/usr/local/bin/rustwain` (+ `rwctl` symlink),
- `rustwain-server.service` / `rustwain-agent.service` with `Delegate=yes`,
  `KillMode=process` semantics as designed (see host variance),
- NetworkManager/networkd drop-ins for `rw*` devices,
- sysctl drop-in `/etc/sysctl.d/90-rustwain.conf`,
- modules-load drop-in for needed modules (`geneve`, `wireguard`, …).
Each step is reversible: `rustwain uninstall` removes exactly what install added (tracked in
an install manifest).

## First server

```bash
rustwain server init --cluster-name prod --pod-cidr fd00:10:42::/48,10.42.0.0/16 \
    --service-cidr fd00:10:96::/112,10.96.0.0/16
```

Steps:
1. **Preflight** (`doctor` checks; fatal ones abort).
2. **PKI**: generate root + intermediates (or import external root via `--ca-cert/--ca-key`
   or `--ca-external` to leave the root offline); server certs for this node.
3. **Store**: initialise single-member Raft cluster, write cluster config (CIDRs, cluster
   ID, feature flags) as the first entries.
4. **Built-in objects**: system namespaces, default priority classes, built-in roles,
   default storage class (local), default policies.
5. Start API, scheduler, controllers.
6. Optionally **also become an agent** (`--with-agent`, default for small clusters): taint
   itself `control-plane:NoSchedule` unless `--allow-workloads`.
7. Output:
   - Admin credential (`~/.config/rustwain/config` written, file 0600).
   - A **join command** for servers and for agents.

## Join tokens

```
rustwain agent join https://10.0.0.10:6443 --token rwjt.9f3b...  --ca-hash sha256:4a1c...
```
- Token = `id.secret`; server stores only a hash (Argon2/HMAC) of the secret.
- Scope: `agent` or `server`; optional node pool, max uses, TTL (default 24 h).
- `--ca-hash` pins the cluster CA public key (like kubeadm's
  `--discovery-token-ca-cert-hash`) – prevents MITM on first contact.
- Alternative identities: cloud instance identity documents (`--join-method aws-iid`),
  TPM attestation `[later]`, or pre-provisioned certs.

## Additional servers
```bash
rustwain server join https://10.0.0.10:6443 --token <server-token> --ca-hash ...
```
1. Authenticate with server token → receive server cert.
2. Join Raft as **learner**; stream snapshot + log.
3. When caught up, promoted to voter (joint consensus).
4. Start API/controllers (controllers only lead if they win leases).

Recommended: 3 servers (tolerates 1 failure) or 5 (tolerates 2).

## Agent join flow
1. Preflight.
2. TLS bootstrap: token → CSR → node cert (see [identity and PKI](../09-security/01-identity-and-pki.md)).
3. Register Node with features; receive pod CIDR block(s); configure dataplane.
4. Ready.

Target: agent join → Ready in < 10 s on a prepared host.

## Configuration file alternative
Everything above can be expressed declaratively:
```toml
# /etc/rustwain/config.toml
role = "agent"
server = "https://10.0.0.10:6443"
token_file = "/run/secrets/rustwain-join"
ca_hash = "sha256:4a1c..."
[node]
labels = { pool = "web" }
taints = []
```
Supports cloud-init / ignition provisioning.

## Declarative cluster lifecycle `[later]`
Cluster API-like management: a management cluster that creates VMs (via node providers)
and bootstraps workload clusters. Builds on the node provider interface from autoscaling.

## Dev mode
`rustwain dev` – single process (server + agent), in-memory or local store, no Raft
quorum, generous defaults, prints a ready-to-use config. For laptops (Linux) and CI.
On macOS, the CLI can start a lightweight Linux VM (via Virtualization.framework through a
small helper `[open]`) – out of scope initially.

## Open questions

- Store the admin credential as a long-lived cert? Better: short-lived cert + OIDC setup
  prompt; but first-run UX needs something that works offline. Lean: 1-year admin cert
  stored locally, with a warning to set up OIDC.
- Server join over a different port/protocol than API? Same port is simpler.

## Build checklist

- [ ] `rustwain install/uninstall` with manifest of changes.
- [ ] `server init`, `server join`, `agent join` flows with preflight.
- [ ] Join tokens (hashed, scoped, TTL) + CA hash pinning.
- [ ] Learner → voter promotion automation.
- [ ] `rustwain dev`.
- [ ] End-to-end test: 3 servers + 3 agents in VMs from zero to running a Deployment.
