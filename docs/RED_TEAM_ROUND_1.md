# Apollo Artifactd red-team round 1

**Verdict: BLOCK RELEASE.** This is an implementation-mode review of the current
`apollo-artifactd` tree against the fixed cutover request. It is a non-converged
round: the implementation has confirmed release-blocking gaps, so no production
approval is claimed.

## Charter and scope

The fixed requirements are a standalone Rust artifact service, complete CAS and
OCI verification, registry pull/push, secure credential handling, rootfs
preparation, pin/lease-safe GC, crash recovery, the versioned Unix API, and
qualified callers with the old Zig importer removed. The review covered the
`apollo-artifactd` source, protocol crate, tests, migration map, and buildd
migration notes. No production service was changed and no remote service was
modified.

## Confirmed findings

### RT-001 — Critical — registry operations are deliberately absent

* Claim / requirement: artifactd must support HTTPS authenticated OCI registry
  pull and push, including verified transfers and idempotent retries.
* Evidence: `src/api/dispatch.rs:144-149` advertises `"registry": false` and
  `"production_qualified": false`; `PULL` and `PUSH` unconditionally return
  `registry operations unavailable`. `Cargo.toml` has no HTTP/TLS/OCI registry
  client dependency. `docs/MIGRATION.md:11` records registry as pending.
* Counterexample: issue a valid version-1 `PULL` or `PUSH` request for a
  digest-pinned reference. The daemon rejects it before any network transfer,
  so a buildd cannot acquire or publish an artifact through the required
  replacement path.
* Impact: the central artifact acquisition/publication path does not exist;
  the requested cutover cannot run production builds.
* Smallest correction: implement registry pull/push behind the generic
  protocol with HTTPS, bounded redirects/timeouts, descriptor and body
  verification, authentication via protected FD/provider, and durable
  resumable operation state. Add authenticated integration tests for success,
  substitution, retry, and failure cases.
* Disposition: **ACCEPTED**. Closing evidence is a real registry test matrix
  and capabilities output with registry enabled.

### RT-002 — High — no production caller cutover or Zig-removal closure

* Claim / requirement: buildd must call artifactd directly; no subprocess bridge,
  Zig importer, or production dependency on the old Artifact implementation
  may remain.
* Evidence: `apollo-buildd/docs/MIGRATION.md:27-30` says the publisher bridge,
  legacy importer, and process scripts are not implemented/retired. The
  artifactd migration map at `docs/MIGRATION.md:17-24` lists direct integration
  and removal as pending. Repository search finds no artifactd client in
  `apollo-buildd` and no completed removal closure.
* Counterexample: run the current buildd publication path and inspect its
  artifact handoff. There is no proven artifactd protocol call, so the build
  result cannot be made durable through this service; old paths remain required
  by the documented migration state.
* Impact: the requested full Rust cutover is not deployed, even if the local
  CAS tests pass.
* Correction: add a buildd-owned generic protocol client and end-to-end receipt
  binding, qualify restart/timeout ambiguity, then remove the old importer and
  process bridge only after callers and release scripts have migrated.
* Disposition: **ACCEPTED**.

### RT-003 — High — registry credential contract is missing

* Claim / requirement: credentials must use a secure generic mechanism and never
  appear in CLI arguments, logs, or artifactd state; artifactd must not depend
  on secretsd.
* Evidence: protocol actions only carry `Pull { reference, platform }` and
  `Push { digest, reference }`; there is no credential-provider or protected FD
  field. Dispatch rejects both operations (`src/api/dispatch.rs:147-149`), so
  there is no implemented admission, redaction, lifetime, or state exclusion to
  test. The CLI accepts only a JSON action and input path (`src/cli.rs:4-17`),
  with no generic protected credential channel.
* Counterexample: a registry requiring bearer auth cannot be used without
  either adding credentials to the action/CLI (violating the requirement) or
  inventing an unimplemented out-of-band mechanism.
* Impact: private registries and the required credential-leakage boundary are
  unsupported.
* Correction: define a credential-provider handle/FD contract, pass it only to
  the registry client, prohibit serialization into operations/state, and test
  process arguments, logs, state, and error responses for secret absence.
* Disposition: **ACCEPTED**.

### RT-004 — Medium — the full qualification suite currently fails due to a
test lifetime bug

* Claim / requirement: property testing must establish that stored identity
  always matches stored bytes.
* Evidence: `tests/cas.rs:9-13` returns `(TempDir, Store)`, but
  `tests/cas.rs:111-114` binds `let (_, mut store) = fixture()`. Dropping the
  `TempDir` deletes the store while `Store` still holds descriptors. Running
  `cargo test` fails `stored_identity_matches_bytes` with `No such file or
  directory`; the other four CAS tests pass.
* Counterexample: `cd apollo-artifactd && cargo test` reproduces the failure;
  the failing proptest case reaches `import_blob` after its backing directory
  has been removed.
* Impact: the advertised test suite is red and does not provide a valid
  property-test signal. This is a test defect, not evidence that CAS integrity
  is broken.
* Correction: keep the TempDir alive in the property test and rerun the full
  suite. Add explicit crash/fault tests instead of relying on this property
  test for recovery coverage.
* Disposition: **ACCEPTED**.

### RT-005 — Medium — required adversarial and operational coverage is absent

* Claim / requirement: native Linux qualification must cover registry,
  concurrent import, crash during import/publish/preparation/GC, disk full,
  protocol abuse, and 100,000/20,000 churn campaigns; dependency audit and
  performance measurements are also required.
* Evidence: the only checked-in tests are `tests/cas.rs` and `tests/oci.rs`.
  `docs/MIGRATION.md:19-20,28` explicitly records crash-boundary tests,
  churn, benchmarks, fuzz/property qualification, and independent approval as
  pending. No `fuzz/`, benchmark, native qualification, registry fixture, or
  fault-injection harness exists under artifactd.
* Counterexample: there is no executable test that kills the process between
  durable intent/effect/completion, exercises registry authentication, or runs
  the requested churn. Those claims therefore cannot be verified from the
  current tree.
* Impact: crash safety, resource bounds under churn, registry behavior, and
  performance remain unverified release gates.
* Correction: add bounded native Linux qualification and fault injection,
  protocol/OCI/archive fuzz targets, concurrency/churn runs, metrics capture,
  `cargo audit`, `cargo deny`, and SBOM review with recorded commands/results.
* Disposition: **ACCEPTED**.

### RT-006 — Medium — single-request server execution permits avoidable
availability starvation

* Claim / requirement: bounded concurrency and resource exhaustion defenses must
  preserve service availability under large valid work.
* Evidence: `src/api/mod.rs:53-61` accepts one socket and immediately calls
  `handle`; `src/api/dispatch.rs` performs imports and preparation synchronously
  on the same loop. A valid regular input may be up to `max_store` at admission
  and a blob up to 4 GiB by default (`src/cas/mod.rs`), while there is no
  operation deadline or cancellation during the blocking read/extraction.
* Counterexample: a same-UID client submits a valid multi-gigabyte import (or a
  maximal decompression/prepare operation) and holds the daemon in that request.
  Other clients cannot be accepted/served until it completes.
* Impact: a trusted or compromised same-user caller can create long service-wide
  stalls. This is bounded work, but not a production availability contract.
* Correction: introduce a small bounded worker pool or admission queue with
  per-operation deadlines/cancellation and explicit saturation responses; keep
  store mutation serialized or transactionally coordinated.
* Disposition: **ACCEPTED**, pending measured capacity and a deliberate
  concurrency policy.

## Attack results where current safeguards held

The local tests and source review provide useful but incomplete evidence for
several boundaries. CAS imports stream through a fixed 64 KiB buffer, hash and
size-check before publication, use private temp files, and publish via a hard
link (`src/cas/import.rs`). Descriptor-relative operations reject foreign
symlinks and non-private files (`src/filesystem.rs`); the foreign-link GC test
passed. OCI graph traversal verifies descriptor digest/size and checks platform
substitution (`src/oci/mod.rs:125-140`); the multi-platform substitution test
passed. Layer preparation rejects absolute/traversal paths, hardlinks, devices,
unsafe symlinks, oversized entries, decompression over-budget, and DiffID
mismatch (`src/prepare/layers.rs:63-130`); the existing malicious archive and
symlink tests passed. Pins and leases protect graph-reachable blobs in the
local GC tests. These are source/test observations only and do not close the
missing registry, caller, crash, or qualification gates.

## Adjudication and traceability

| Requirement | Enforcement observed | Verification | Status |
| --- | --- | --- | --- |
| Streaming CAS, digest/size, atomic private publication | `cas/import.rs`, `filesystem.rs` | CAS unit/property tests (property currently broken) | partial |
| OCI index/manifest/config/layers and platform graph | `oci/mod.rs` | `tests/oci.rs` | partial |
| Safe layers, DiffID, whiteouts, rootfs output | `prepare/layers.rs`, `prepare/tree.rs` | limited fixture tests | partial |
| Pins, leases, graph-aware GC | `cas/references.rs`, recovery | basic CAS/OCI tests | partial |
| Crash recovery and power-loss boundaries | recovery code exists | no kill/fault-injection qualification | unverified |
| Registry pull/push/auth/retries | dispatch explicitly rejects | no client/tests | missing |
| Buildd direct caller and Zig removal | migration docs mark pending | no caller/cutover proof | missing |
| Resource/churn/performance qualification | no harness/measurements | not run | missing |

## Next correction order

1. Correct the property-test fixture lifetime and establish a reproducible green
   baseline.
2. Implement and test the generic registry/credential contract, including
   durable idempotency and secret non-persistence.
3. Integrate buildd over the artifactd protocol; qualify restart and timeout
   ambiguity; remove the old importer/process bridge only after that evidence.
4. Add fault-injection, protocol/OCI/archive fuzzing, Linux concurrency,
   disk-full, crash recovery, and churn qualification on both x86 and ARM
   hosts.
5. Measure idle/load RSS, CPU, threads, FDs, throughput, and p50/p95/p99; then
   rerun an independent red-team round. Two clean complete rounds are required
   before release approval.

## Round status

Red Team rounds: 1 (this incomplete/blocked round)
Simplifier rounds: 0
Evidence-verification passes: repository and local test inspection
Consecutive clean rounds: 0
Critical findings: 1
High findings: 2
Medium findings: 3
Accepted findings: 6
Rejected findings: 0
Needs verification: 0

Final verdict: **BLOCK RELEASE**. The correct completion token is
`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL` until the accepted findings and all fixed
qualification gates are closed.
