# Authentication and Authorization

**K8s analogue:** authentication (x509 client certs, bearer tokens, OIDC, webhook token
review, structured authentication config), authorization (RBAC: Role, ClusterRole,
RoleBinding, ClusterRoleBinding; Node authorizer; webhook; ABAC), impersonation,
SelfSubjectAccessReview, `kubectl auth can-i`.

## Authentication

Supported methods:
1. **Client certificates** (cluster CA) – for agents, servers, break-glass admin.
2. **OIDC** – human users via an IdP (Okta, Google, Keycloak, Dex). Multiple issuers
   supported, claim mappings (username, groups) configured per issuer (Kubernetes'
   structured authentication config). `rwctl login` does the OIDC device/browser flow and
   caches tokens.
3. **Workload tokens** (bound JWTs).
4. **Short-lived user certificates** issued after OIDC login (`rwctl login --cert`) – useful
   where token refresh is awkward.
5. Static tokens: **not supported** (Kubernetes' `--token-auth-file` is a footgun).

Result: `UserInfo { name, uid, groups, extra }`.

Anonymous access: disabled by default except for health and OIDC discovery endpoints.

## Authorization

### RBAC (kept, with improvements)
Kubernetes RBAC is solid: additive permissions, Roles (namespaced) / ClusterRoles,
bindings to users/groups/service accounts, verbs × resources × names.

Pain points:
- Verbose; aggregation and wildcards confusing.
- **Escalation paths are subtle**: `create pods` in a namespace effectively grants use of any
  service account and secret in that namespace (mount it into a pod) – many users don't
  realise. `escalate`/`bind`/`impersonate` verbs, `nodes/proxy` → exec everywhere, etc.
- No deny rules.
- Hard to answer "who can do X?" (requires external tools like `rbac-lookup`, `kubectl-who-can`).

Rustwain RBAC `[proposed]`:
- Same core model (roles + bindings), with:
  - **Tenant-scoped roles** in addition to namespace and cluster scope.
  - **Deny rules** at cluster scope only (for guardrails, e.g. "deny secrets/get to group
    contractors across all tenants") – evaluated before allows.
  - **Built-in role catalogue** with clear names: `viewer`, `editor`, `admin`,
    `tenant-admin`, `cluster-admin`, `node`, `scheduler`, `controller:<name>` – each documented
    with its escalation implications.
  - **Analysis tools**: `rwctl auth can <subject> <verb> <resource>`,
    `rwctl auth who-can <verb> <resource>`, `rwctl auth explain <subject>` (effective
    permissions, with "this also implies…" escalation warnings – e.g. "can create pods in
    `shop` → can read all secrets in `shop`").
- Authorization decision cache keyed by (user, groups hash, verb, resource, ns) with
  invalidation on RBAC object changes.

### Node authorizer
Agents can only:
- Read pods bound to them, and configs/secrets/claims/service accounts referenced by those
  pods (graph-based authorization – Kubernetes' Node authorizer maintains a graph of
  pod→secret/configmap/pvc edges).
- Write status for their own node and pods.
- Create events about their pods.
Implemented with the store's reference indexes (pod → referenced objects).

### Attribute/expression-based authorization `[later]`
For policies like "users in group X may only create pods with label team=X" – that's
admission (validation policy with access to `request.userInfo`), not authorization. Keep
the split: RBAC answers "may this subject perform this verb on this resource?", admission
answers "is this specific object acceptable?".

## Impersonation
`rwctl --as=user --as-group=g`: requires `impersonate` permission; audited with both
identities. Useful for testing RBAC.

## Exec/attach/port-forward/files authorization
Separate verbs (`pods/exec create`, `pods/files get`), never implied by `get pods`. Node
debug requires a high-privilege verb.

## Break-glass
A cluster-admin client certificate generated at bootstrap, stored offline. Usage is
audited prominently. Optionally time-limited "emergency access" tokens issued by
`rwctl` with root CA key.

## Open questions

- Deny rules: limited to cluster scope to keep reasoning tractable – enough?
- Should `create pods` require explicit `use` permission on the service account the pod
  runs as (closing the escalation path)? Strong lean **yes** (`serviceaccounts/use`), with a
  default role granting it for the namespace's default SA.

## Build checklist

- [ ] Authenticator chain (certs, OIDC multi-issuer, workload JWTs).
- [ ] RBAC types + evaluator + decision cache + deny rules.
- [ ] Node authorizer using reference indexes.
- [ ] `serviceaccounts/use` check in admission.
- [ ] `rwctl auth can / who-can / explain`.
