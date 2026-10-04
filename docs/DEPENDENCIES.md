# Dependency acceptance record

This is a conditional implementation review, not production approval. Cargo.lock records the exact dependency graph; the production and fuzz workspaces each have an independent lockfile. No Apollo sibling path or Git dependency is accepted. Production path dependencies are the package-owned protocol crate and the vendored third-party OCI client; neither depends on an Apollo sibling.

| Mechanism | Accepted crate | Policy imposed by artifactd / remaining limits |
|---|---|---|
| SHA-256 | sha2 0.10.9 | Canonical digest parsing; streaming hash; compare expected size/digest before publication; rehash existing files. No local SHA implementation. |
| Linux FD/syscalls | rustix 1.1.5 | NOFOLLOW, descriptor-relative opens, owned FD lifetime, CLOEXEC, SO_PEERCRED, bounded SCM_RIGHTS and poll deadlines. No hand-written syscall wrappers. |
| Capability filesystem | cap-std 4.0.3 | Private owner root, anchored handles, explicit component/link policies; reopen O_PATH directory handles read-only for fsync. Same-owner hostile processes remain outside the qualified boundary. |
| Transaction database | redb 4.3 | Capability-opened private file, Immediate transaction durability, 8 MiB cache, schema version, bounded metadata and persisted mutation intents. No custom database. Legacy SQLite stores fail closed; conversion and corruption qualification remain open. |
| OCI models | oci-spec 0.9.0 | Standard structs; locally enforce graph/media/digest/size/platform/DiffID relationships and metadata limits. Unsupported descriptor extensions are rejected. |
| Tar | tar 0.4.46 | Raw entries only; reject extension headers before library buffering, links/devices/path escape and oversized content; strict trailing terminator policy; never use unconstrained unpack. Safe GNU/PAX portability remains missing. |
| Compression | flate2 1.1.10 / zstd 0.13.3 | Bound decompressed reads and total output; verify DiffIDs before applying layer effects. Compression mechanisms are library code. Native zstd requires a C toolchain. |
| JSON | serde / serde_json 1.0.151 | Bounded packet/metadata reads and graph depth/descriptor budgets. JSON is a versioned local protocol; no protobuf implementation is introduced. |
| CLI/IDs | clap / uuid | Generic identifiers; no credentials in CLI arguments and no shell execution. Registry credentials use a protected FD and are excluded from journal entries and errors; full credential/security qualification remains open. |
| Properties/fuzz | proptest / libfuzzer-sys | Test-only; separate fuzz workspace. Smoke results are not full fuzz qualification. |

## Registry client decision

`oci-client` 0.18.0 supplies OCI registry, HTTP and TLS mechanisms. Its upstream defaults were insufficient for the required response bounds, redirect and authentication policies. The package therefore vendors an Apache-2.0 fork with explicit bounded bodies, request deadlines, secure redirects and exact authentication realm authority policy. See `vendor/oci-client/ARTIFACTD_PATCH.md` for provenance and public client-boundary tests. This fork creates an ongoing security maintenance obligation; upstream changes must be reviewed before updates.

`reqwest` and `rustls` remain responsible for HTTP and TLS. Artifactd requires HTTPS, uses the platform trust store or an explicit private-provider CA, and does not enable invalid-certificate acceptance. Credential providers are size-bounded regular files restricted to their owner; errors from remote requests are sanitized. Authenticated native registry qualification is recorded separately from upstream client tests.

The locked graph includes `webpki-root-certs` 1.0.9 under CDLA-Permissive-2.0. The license was reviewed before allowlisting: it permits use/modification/sharing of the certificate data and requires the license text to accompany shared data. `licenses/CDLA-Permissive-2.0.txt` retains the exact crate license for distribution. Package distributions must include this notice and the vendored OCI client's Apache license. No advisory exception is used.

## Tool evidence

- `cargo-audit.json` and `cargo-audit-fuzz.json`: zero vulnerability results, no advisory warnings.
- `cargo-deny.txt`: advisories/bans/licenses/sources pass. Duplicate dependency versions remain warnings; no ignored advisory was introduced.
- `cargo-tree.txt`: locked production graph, including transitive security-relevant mechanisms.
- `sbom.cdx.json`: metadata-derived CycloneDX 1.5 components, Cargo purls, declared license expressions and dependency edges. Official JSON-schema and dependency-reference validation passed (`sbom-validation.txt`). It is an inventory, not a binary provenance/signature document; The original 167-component inventory is preserved as `sbom-baseline-3a42141.cdx.json`; the current 293-component inventory is regenerated from the lockfile and validated in `sbom-validation-redb-registry.txt`.

Dependency tooling cannot replace archive, filesystem, recovery and protocol security testing. Third-party source review here inspected the mechanisms and bounded integration points above; it does not constitute a complete audit of every transitive crate or a red-team release approval.

Current integration tooling uses `cargo-audit-redb-registry.json`, `cargo-deny-redb-registry.txt`, and `cargo-tree-redb-registry.txt`. Baseline filenames above remain historical evidence and do not qualify new dependencies.
