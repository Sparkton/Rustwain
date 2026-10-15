# Supply Chain Security

**K8s analogue:** image signature verification via admission (Sigstore policy-controller,
Kyverno verifyImages, Connaisseur), Notary v2/Notation, SBOMs, SLSA provenance,
Kubernetes' own signed releases (cosign).

## Two supply chains

1. **Workload images** users run on Rustwain.
2. **Rustwain itself** (our binaries and images).

## Workload image verification

### Policy
```yaml
kind: ImagePolicy
spec:
  match: { tenants: [prod] }
  rules:
    - images: ["ghcr.io/acme/*"]
      require:
        signatures:
          - keyless: { issuer: "https://token.actions.githubusercontent.com",
                       subject: "https://github.com/acme/*/.github/workflows/release.yml@refs/heads/main" }
        provenance: { slsaLevel: 3 }        # optional attestation check
    - images: ["*"]
      action: Deny
```

### Where to verify
- **Admission** (API server) – verify signature for the resolved digest when the pod (or
  workload template) is created. Needs registry access from servers (fetch signatures via
  OCI referrers API).
- **Node** (agent, at pull time) – verify again before running (defence against a
  compromised API path or TOCTOU between admission and pull; digests are pinned so the
  bytes can't change, but the check also catches policy bypass).
- Rustwain: admission verification **records the verified digest + policy in the pod
  status/annotation signed by the server**; the agent checks the record and the digest
  match – no second network verification needed, unless configured for paranoid mode.

### Implementation
- Sigstore in Rust: `sigstore-rs` crate (cosign signature verification, keyless with Fulcio
  certs and Rekor transparency log verification).
- Notation (Notary v2) signatures: verify via a Rust implementation of the JWS/COSE envelope
  `[later]`.
- Cache verification results per digest+policy hash.

### Integrity at runtime `[later]`
fs-verity/composefs (see [filesystems](../01-linux-foundations/04-filesystems.md)): the
node enforces that file contents match the signed image digest at read time – protects
against on-node tampering of unpacked layers.

## Rustwain's own supply chain
- Reproducible builds (`cargo` with locked deps, `SOURCE_DATE_EPOCH`, fixed toolchain via
  `rust-toolchain.toml`).
- Dependency policy: `cargo-deny` (licenses, advisories, banned crates, duplicate
  versions), `cargo-audit` in CI, `cargo-vet` for third-party crate audits `[proposed]`.
- Minimise dependencies in security-critical crates (supervisor, container setup, unpacker).
- Release artifacts: signed (cosign keyless from CI), SBOM (CycloneDX via
  `cargo-cyclonedx`), SLSA provenance.
- Agents verify upgrade binaries' signatures before switching (see
  [upgrades](../10-cluster-operations/02-upgrades-and-compat.md)).

## Open questions

- sigstore-rs maturity for keyless verification with offline/air-gapped trust roots.
- Admission-time verification latency (registry fetch) – cache and async pre-verification
  for workload templates on apply.

## Build checklist

- [ ] ImagePolicy kind; admission verifier with cache.
- [ ] Signed verification record consumed by agent.
- [ ] CI: cargo-deny, cargo-audit, SBOM, signed releases.
