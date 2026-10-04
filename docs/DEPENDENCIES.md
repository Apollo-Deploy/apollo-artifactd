# Dependency acceptance record

This is a conditional implementation review, not production approval. Cargo.lock records the exact dependency graph; the production and fuzz workspaces each have an independent lockfile. No Apollo sibling path or Git dependency is accepted. The sole production path dependency is the package-owned protocol crate.

| Mechanism | Accepted crate | Policy imposed by artifactd / remaining limits |
|---|---|---|
| SHA-256 | sha2 0.10.9 | Canonical digest parsing; streaming hash; compare expected size/digest before publication; rehash existing files. No local SHA implementation. |
| Linux FD/syscalls | rustix 1.1.5 | NOFOLLOW, descriptor-relative opens, owned FD lifetime, CLOEXEC, SO_PEERCRED, bounded SCM_RIGHTS and poll deadlines. No hand-written syscall wrappers. |
| Capability filesystem | cap-std 4.0.3 | Private owner root, anchored handles, explicit component/link policies; reopen O_PATH directory handles read-only for fsync. Same-owner hostile processes remain outside the qualified boundary. |
| Transaction database | rusqlite 0.38.0 / libsqlite3-sys 0.36.0 | Bundled SQLite, FULL synchronous WAL, schema version, bounded metadata, persisted mutation intents, no custom database. SQLite integrity checks do not alone prove semantic reachability. |
| OCI models | oci-spec 0.9.0 | Standard structs; locally enforce graph/media/digest/size/platform/DiffID relationships and metadata limits. Unsupported descriptor extensions are rejected. |
| Tar | tar 0.4.46 | Raw entries only; reject extension headers before library buffering, links/devices/path escape and oversized content; strict trailing terminator policy; never use unconstrained unpack. Safe GNU/PAX portability remains missing. |
| Compression | flate2 1.1.10 / zstd 0.13.3 | Bound decompressed reads and total output; verify DiffIDs before applying layer effects. Compression mechanisms are library code. Native zstd requires a C toolchain. |
| JSON | serde / serde_json 1.0.151 | Bounded packet/metadata reads and graph depth/descriptor budgets. JSON is a versioned local protocol; no protobuf implementation is introduced. |
| CLI/IDs | clap / uuid | Generic identifiers; no credentials in CLI arguments and no shell execution. Registry credential transport is absent, so this does not qualify credential handling. |
| Properties/fuzz | proptest / libfuzzer-sys | Test-only; separate fuzz workspace. Smoke results are not full fuzz qualification. |

## Registry client decision

`oci-client` 0.18.0 was evaluated from its crate source as an established OCI registry implementation. It was not added. Its client configuration does not expose the required redirect/auth-realm policy and total request timeout; some manifest/token/error paths buffer entire response bodies. A reviewed bounded integration or maintained library patch is required before accepting registry support. Reimplementing HTTP/TLS or the registry protocol in artifactd is not an accepted workaround.

## Tool evidence

- `cargo-audit.json` and `cargo-audit-fuzz.json`: zero vulnerability results, no advisory warnings.
- `cargo-deny.txt`: advisories/bans/licenses/sources pass. Duplicate dependency versions remain warnings; no ignored advisory was introduced.
- `cargo-tree.txt`: locked production graph, including transitive security-relevant mechanisms.
- `sbom.cdx.json`: metadata-derived CycloneDX 1.5 components, Cargo purls, declared license expressions and dependency edges. Official JSON-schema and dependency-reference validation passed (`sbom-validation.txt`). It is an inventory, not a binary provenance/signature document; registry implementation is not present.

Dependency tooling cannot replace archive, filesystem, recovery and protocol security testing. Third-party source review here inspected the mechanisms and bounded integration points above; it does not constitute a complete audit of every transitive crate or a red-team release approval.
