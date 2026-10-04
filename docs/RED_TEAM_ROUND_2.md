# Apollo Artifactd red-team round 2

Date: 2026-10-04

This was an independent adversarial review of the revised standalone Rust
implementation. Production code was not changed. The review concentrated on
the replay, duplicate-import, live-graph GC, descriptor-relative fsync, and
SIGKILL recovery changes. The verdict is **BLOCK RELEASE**.

## Evidence collected

On the review host (`Darwin 27.2.0 arm64`), `cargo test --all-targets` passed:
6 CAS/OCI tests and 4 recovery tests, including
`sigkill_during_import_never_resolves_partial_content`. `cargo deny check` and
`cargo audit` both exited zero. This is useful unit/property evidence, but it
does not qualify the Linux daemon boundary: `tests/api.rs` is guarded by
`#![cfg(target_os = "linux")]` (line 1), and the test runner reported 0 API
tests on this host. Read-only SSH checks confirmed `apollo-node-01` is Linux
aarch64 and `tihan-apollo` is Linux x86_64, but this review found no committed
native qualification result that closes the required matrix.

## Findings

### RT2-001 — Critical — PULL and PUSH remain an explicit production stub

`src/api/dispatch.rs:170-175` reports `registry: false` and
`production_qualified: false`, and both `Action::Pull` and `Action::Push`
unconditionally return `registry operations unavailable`. This leaves HTTPS,
authentication, bounded redirects, streaming transfer, digest-pinned pulls,
manifest verification, retry idempotency, and ordered pushes absent. The
revised CAS/recovery changes do not alter this blocker. `docs/MIGRATION.md:11`
also records registry support as pending.

Impact: an artifact that exists only in a registry cannot be acquired or
published through the required service, so the central buildd-to-artifactd
cutover cannot work.

Required closure: implement a bounded authenticated registry client and prove
pull/push behavior against an adversarial registry matrix, including auth
failure and digest/tag race cases. Capabilities must change only after those
tests pass.

### RT2-002 — High — caller cutover and Zig removal remain open

The migration map still marks direct publisher, BuildKit transport, Node, and
external client integration as pending (`docs/MIGRATION.md:16-19,24`). No
artifactd caller implementation or proof of removal of the Zig importer and
process-launch paths was found in this package. The repository's own primary
review also states that BuildKit/Node/release callers remain Zig-dependent
(`docs/PRIMARY_REVIEW.md:9-11`).

Impact: even a locally correct daemon is not the production artifact path, and
the requested zero-Zig cutover is not demonstrated.

Required closure: migrate every production caller to `crates/artifactd-protocol`,
remove the Zig implementation/importer/process bridge, and show a clean build
with no sibling Apollo package dependency or Zig requirement.

### RT2-003 — High — no secure registry credential contract exists

The protocol actions carry only registry reference/platform or digest/reference;
there is no credential FD, protected provider handle, or equivalent generic
credential input. Since dispatch rejects PULL/PUSH, no implementation currently
proves that credentials are excluded from operation state, logs, argv, or
persistent artifacts. `docs/MIGRATION.md:28` explicitly lists the credential
checked API as incomplete.

Impact: adding registry transport without first defining this boundary risks
credential leakage and would not satisfy the API requirement.

Required closure: define an opaque credential-provider/FD contract, enforce
permissions and lifecycle, redact all errors/logs, and test bearer/basic auth
success and failure without credentials appearing in state or process listings.

### RT2-004 — Medium — operation replay is still only bounded, not durable

`src/api/dispatch.rs:103-105` deletes completed and failed operation rows older
than roughly 4096 sequence positions, and the comment explicitly says old IDs
may be re-executed. The replay check at lines 57-75 therefore ceases to protect
an old `OperationId` after retention pruning. A successful timeout-after-commit
followed by more than 4096 operations can replay the effect under the same ID;
this is especially material for future registry push/publish effects and for
any mutation whose idempotency depends on durable operation identity.

The current local tests exercise immediate replay, not replay after the pruning
boundary. The bounded window may be acceptable only with an explicit caller
retry-horizon contract and a regression test proving the horizon; neither is
present in the protocol or migration evidence.

Required closure: retain compact durable mutation outcomes for the supported
retry horizon, or specify and enforce a protocol expiration contract. Add a
test that commits an effect, prunes history, and verifies the documented replay
behavior.

### RT2-005 — Medium — required native qualification and churn/performance evidence is absent

The revised repository contains fuzz-target source files and local examples,
but no committed report demonstrating the requested Linux x86_64 and aarch64
matrix, 100,000 blob operations, 20,000 OCI pin/unpin/GC lifecycles, disk-full
behavior, crash during preparation/GC/publish, registry auth, or RSS/CPU/
threads/FD and p50/p95/p99 measurements. `docs/PRIMARY_REVIEW.md:11,15,20`
and `docs/MIGRATION.md:19-20` continue to mark these gates incomplete.

Impact: bounded memory/FD growth, incremental GC, and crash safety under real
Linux filesystem behavior remain unverified. Passing host-side unit tests is
not evidence for the required daemon boundary or ARM behavior.

Required closure: run isolated, reproducible qualification suites over SSH on
`tihan-apollo` and `apollo-node-01`, retain raw logs and metrics, and include
fault-injection results for every required mutation boundary.

### RT2-006 — Low — dependency policy is green but duplicate-version warnings remain

`cargo deny check` and `cargo audit` exit zero, but cargo-deny reports many
duplicate transitive versions (including `io-lifetimes`, `syn`, and multiple
`windows-*` crates). This is not a release blocker by itself, but the SBOM and
security review should explain accepted duplicates and verify that no
security-critical mechanism is split across incompatible versions.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL` — **do not approve release**. The revised
duplicate-import verification, live-graph GC checks, O_PATH fsync correction,
and SIGKILL import recovery test are meaningful improvements and the available
local tests pass. They do not close the registry, secure credential contract,
caller cutover/Zig removal, durable replay policy, or native qualification
gates. A subsequent round must re-test those closures and the complete Linux
matrix before any production-complete verdict.
