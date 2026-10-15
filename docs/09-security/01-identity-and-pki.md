# Identity and PKI

**K8s analogue:** cluster CA(s) (kubernetes CA, front-proxy CA, etcd CA), kubelet TLS
bootstrap and CSR API, certificate rotation (`rotateCertificates`), service account
signing keys, bound service account tokens, ClusterTrustBundles, cert-manager,
SPIFFE/SPIRE.

## Identities in the system

| Principal | Identity form | Credential |
|---|---|---|
| Server (control plane) | `server:<name>` | X.509 cert (server + client usage) |
| Agent (node) | `node:<name>` | X.509 client cert; server cert for its stream endpoint (if any) |
| Human user | OIDC subject / cert CN | OIDC token, or short-lived client cert |
| Workload (pod) | `spiffe://<cluster>/ns/<ns>/sa/<sa>` | X.509 SVID (mTLS) and/or JWT (API access, external federation) |
| Controller/operator | Service account (it's a workload) | as workload |

## Certificate authorities

Kubernetes kubeadm clusters have several CAs (cluster, front-proxy, etcd) and many certs
with **1-year expiry by default** – "my cluster died because certificates expired" is a
rite of passage.

Rustwain:
- **Root CA** (offline-capable): long-lived (10 y), used only to sign intermediates.
  Private key can live outside the cluster (HSM/file on an admin machine) – or inside for
  simple setups.
- **Intermediate CAs** (online, held by servers, encrypted at rest in the store with the
  cluster KEK):
  - `control-plane-ca`: servers, agents, users' client certs.
  - `workload-ca`: workload SVIDs.
- Automatic, continuous rotation:
  - Leaf certs: **short-lived** (agents 7 days, servers 30 days, workloads 1–24 hours),
    renewed at 2/3 of lifetime. Expiry becomes a non-event because renewal is constant.
  - Intermediates: rotated yearly with overlap (both trusted during transition) via
    **trust bundles** distributed to all parties.
- `rwctl pki status`: every cert's expiry, upcoming rotations; alerts if renewals fail.

Crates: `rustls` (TLS), `rcgen` (cert generation), `x509-parser`/`x509-cert` (parsing),
`ring`/`aws-lc-rs` (crypto; aws-lc-rs for FIPS mode option).

## Agent bootstrap (TLS bootstrapping)
1. Join token (short-lived, scoped `join:<pool>`, single- or multi-use) or platform
   attestation (cloud instance identity document, TPM EK attestation `[later]`).
2. Agent generates key locally → CSR with `node:<name>`.
3. Server verifies token/attestation, checks the name policy (e.g. names must match the
   cloud instance ID), signs.
4. Renewal: agent submits CSR authenticated with its current cert; auto-approved for the
   same identity.
5. Revocation: node removal adds the cert serial to a revocation list distributed to
   servers (short lifetimes bound the exposure anyway).

## Workload identity

### Why both X.509 and JWT
- **X.509 SVIDs** for mTLS between workloads (mesh layer, see
  [service mesh](../05-networking/10-service-mesh.md)) and to the API.
- **JWT** tokens (bound to pod UID, audience-scoped, short-lived) for: calling the API
  server via HTTP gateway, federating with external systems (AWS IAM OIDC federation, GCP
  Workload Identity, Vault JWT auth) – Kubernetes' projected service account tokens.

### Issuance
- The agent requests SVIDs/JWTs for its pods from the server (agent is authorized only for
  pods bound to it – node authorizer) – the agent proves the pod exists on it.
- Delivered via projected volume (files, atomically rotated) or a local **Workload API**
  socket (SPIFFE Workload API compatible `[proposed]`) – lets SPIFFE-aware libraries fetch
  and auto-rotate.
- Kubernetes pre-1.22 mounted **long-lived, non-expiring** service account tokens as
  Secrets into every pod by default – a massive credential-leak risk. Rustwain: no token
  mounted unless requested; all tokens bound + expiring.

### OIDC issuer for federation
Servers expose `/.well-known/openid-configuration` and JWKS for workload JWTs, so cloud
IAM can trust cluster workloads (like EKS IRSA/GKE WI). Signing keys rotate with overlap.

## mTLS everywhere in the control plane
- Client → API: TLS (server cert) + client auth (cert or token).
- Agent ↔ API: mTLS.
- Server ↔ server (Raft): mTLS with `server:` identities only.
- API → webhooks/aggregated APIs: mTLS with workload identity verification.

## Key storage
- Node and server private keys: files 0600 under `/var/lib/rustwain/pki`, optionally
  TPM-sealed `[later]`; never in the API store.
- CA private keys: encrypted in store with KEK, or external KMS/HSM (PKCS#11 `[later]`).

## Open questions

- Single root vs separate roots for control plane and workloads (separation lets workload
  trust be federated without exposing control-plane trust).
- FIPS mode via aws-lc-rs – when?

## Build checklist

- [ ] PKI module: CA hierarchy, issuance, rotation scheduler, trust bundle distribution.
- [ ] CSR flow for nodes; join tokens; auto-renewal.
- [ ] Workload SVID/JWT issuance via agent; projected files + Workload API socket.
- [ ] OIDC discovery/JWKS endpoints.
- [ ] `rwctl pki status/rotate`.
