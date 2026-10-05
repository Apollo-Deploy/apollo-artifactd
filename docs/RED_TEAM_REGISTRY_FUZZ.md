# Scoped red-team review: registry response fuzz target

Date: 2026-10-05
Scope: `fuzz/fuzz_targets/registry_responses.rs` and its fuzz-workspace dependency/corpus changes.
Verdict: **scoped review passed with qualification gaps; no source-level finding**. This is not a release approval and is not evidence of HTTP, authentication, TLS, redirect, native Linux, or long-duration fuzz qualification.

## Reviewed inputs

- `fuzz/fuzz_targets/registry_responses.rs` SHA-256: `3d520f791719262155946ae44a44088284a47e2dd9d663e6215ff251f9e34ec8`
- `fuzz/Cargo.toml` SHA-256: `400d69556678b4b94caa988fbfb9d718d79f474cc308da2681213a06f6bebbfd`
- `fuzz/Cargo.lock` SHA-256: `8070b6a85b4da86c020c1e8fcb10abbd63513ebef99076d1727fc9a53c92f255`
- `vendor/oci-client` is used through its public `OciManifest` and `OciEnvelope` serde types. The direct fuzz-workspace dependency does not add a production runtime dependency or a production test seam.
- Five public seeds are present: `auth-error` (92 bytes), `index` (88), `manifest` (246), `nested-error` (71), and `unknown-error` (45).

The target rejects inputs over 65,536 bytes before either decoder. For each successfully decoded manifest and error envelope it serializes, reparses, and checks semantic JSON value equality; manifests additionally check the derived content type. `unwrap` is an intentional crash oracle if a successfully decoded public value cannot round-trip through its serializer.

## Coverage assessment

The target exercises the vendored OCI client's public manifest union (`OciManifest`, image and image-index variants) and registry error envelope parser. This is useful coverage for parser/serializer regressions and malformed JSON handling. The existing `protocol_oci` target separately exercises the `oci-spec` `ImageManifest`, `ImageIndex`, and `ImageConfiguration` deserializers used by artifactd's graph validation. Digest/config fuzz targets are therefore not required merely to obtain parser coverage, provided those existing targets remain in the fuzz set.

The production registry path is stricter than either decoder alone. `src/oci/mod.rs` parses `oci-spec` values and then enforces schema version 2, descriptor media types, graph relationships, platform/config agreement, layer limits, and DiffID relationships. The new target does not assert those policies. A manifest with an invalid digest, size, schema version, media type, duplicate/ambiguous graph fields, or substitution relationship can still be a valid serde round-trip. This is a coverage limitation, not a demonstrated vulnerability in the target.

The 64 KiB check bounds the libFuzzer input, but it is not a proof that decoder allocations are bounded to 64 KiB: nested JSON strings, vectors, and maps can expand memory. The target also does not exercise HTTP framing, response status/header interpretation, authentication, credential handling, TLS verification, redirect policy, upload/download streaming, timeout behavior, or registry substitution resistance. Those require live-server or protocol-boundary tests.

## Evidence and remaining gate

`docs/fuzz-registry-responses-smoke-20261005.log` records a successful release build of the fuzz workspace (`Finished release profile`); it does not contain a `Running` libFuzzer line or a completed fuzz campaign. No native Linux or long-duration campaign is claimed here. The five-seed corpus is present and hashed above, but seed execution and crash-corpus triage remain separate qualification work.

Recommended follow-up is to retain this target alongside `protocol_oci`, and add a policy-level property target or integration corpus for accepted/rejected OCI graph invariants if full registry-validation fuzz evidence is required. No mandatory scoped defect was found in the reviewed implementation.

## Later execution evidence

The review above preceded completion of the smoke process. The retained smoke log now includes the actual libFuzzer run and terminal success: 101,225 executions in 31 seconds, with bounded 64 KiB input and 1,024 MiB RSS limit. See REGISTRY_FUZZ_QUALIFICATION.md for the final scope. This supersedes only the earlier absence of completed local smoke evidence; native Linux, long-duration and HTTP/auth/TLS qualification gaps remain open, and the review verdict is not a global release approval.
