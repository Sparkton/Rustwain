# Workload Security Policy

**K8s analogue:** Pod Security Standards (Privileged, Baseline, Restricted) and Pod
Security Admission (enforce/audit/warn labels per namespace), PodSecurityPolicy (removed
in 1.25), securityContext fields.

## Security levels

Three levels, like Pod Security Standards (they're well thought through), but with
Rustwain's stronger defaults baked in.

| Control | `privileged` | `baseline` (default) | `restricted` |
|---|---|---|---|
| User namespace | optional | **required** (unless node lacks support → pod rejected or policy fallback) | required |
| Run as non-root (in userns) | any | any (root in userns is not host root) | required (`runAsNonRoot`) |
| Capabilities | any | default set + `NET_BIND_SERVICE`; adding beyond allow-list denied | drop ALL; only `NET_BIND_SERVICE` addable |
| seccomp | any | RuntimeDefault required (Unconfined denied) | RuntimeDefault or stricter |
| no_new_privs | any | required | required |
| Host namespaces (net/pid/ipc) | allowed | denied | denied |
| hostPath volumes | allowed | denied | denied |
| Host ports | allowed | denied (or allow-list) | denied |
| Privileged containers | allowed | denied | denied |
| AppArmor/SELinux overrides | any | only default/runtime profiles | same |
| `/proc` mount type | any | default (masked) | default |
| Sysctls | any | safe set only | safe set only |
| Volume types | any | all except hostPath | config, secret, scratch, claim, projected, image, ephemeral |
| Read-only root fs | – | – | required `[proposed]` |
| Nested userns | allowed | denied unless `allowNestedUserns` permission | denied |
| Isolation class | any | any | any (tenant may force microvm) |

`baseline` in Rustwain is roughly Kubernetes `restricted` minus the non-root requirement,
because user namespaces make container root safe(r).

## Enforcement
- Level set per **tenant** (default) with per-namespace overrides (only stricter, unless
  cluster admin).
- Modes per namespace: `enforce`, `warn` (admission warnings returned to client), `audit`
  (audit annotations) – identical to Pod Security Admission, which works well.
- Evaluated in admission on pods **and workload templates** (Deployment create fails fast
  instead of creating a Deployment whose pods are all rejected – a Kubernetes UX gap: PSA
  warns on workloads but enforces only on pods).
- Exemptions: by namespace (`rustwain-system`), by user (cluster admins), audited.

## Version pinning
Levels are versioned (`baseline@v1`, `baseline@latest`) so that tightening a level in a
new Rustwain release doesn't break running workloads unexpectedly (PSA does this with
`enforce-version`).

## Beyond levels: fine-grained policies
Organisation-specific rules (required labels, allowed registries, resource requirements)
are ValidationPolicies (see [admission and policy](../07-control-plane/07-admission-and-policy.md)).

## Open questions

- Default level for new tenants: `baseline` (proposed) – compatible with most images
  because root-in-userns is allowed.
- Read-only rootfs in `restricted` breaks many images; keep or move to lint?

## Build checklist

- [ ] Level definitions as data (versioned) + evaluator.
- [ ] Admission plugin with enforce/warn/audit modes on pods and templates.
- [ ] Test matrix: each control × each level.
