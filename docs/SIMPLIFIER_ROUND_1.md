# Simplifier review, round 1

**Mode:** implementation review, independent simplifier pass

**Baseline:** standalone `apollo-artifactd` source at the 2026-10-04 workspace revision. This pass did not modify production code, tests, fixtures, state, or deployment artifacts. The x86 and arm qualification hosts (`tihan-apollo` and `apollo-node-01`) were not used because this pass is static plus local test evidence; no remote qualification claim is made.

**Verdict:** `Block release`. The current implementation contains useful CAS/OCI/rootfs mechanisms, but fixed user requirements remain missing. Several persisted fields and operation records can be removed or narrowed once the required durability contract is made explicit. This document is not a production approval.

## Findings

### S1 — Registry surface is an explicit stub, not a simplification

**Priority:** high  **Confidence:** high  **Disposition:** ACCEPTED as an incomplete requirement, not a simplifier change

The protocol exposes `PULL` and `PUSH`, but dispatch rejects both with `registry operations unavailable` (`src/api/dispatch.rs:75`). Capabilities reports `registry: false` and `production_qualified: false` (`src/api/dispatch.rs:74`). The fixed requirements require HTTPS, authentication, bounded redirects, streaming transfer, digest-pinned pulls, manifest verification, idempotent retries, and ordered pushes. This cannot be closed by deleting the API; the API is required. Implement and qualify a bounded registry adapter, then remove the explicit incomplete path.

### S2 — The requested cutover and caller migration are absent

**Priority:** high  **Confidence:** high  **Disposition:** ACCEPTED as an incomplete requirement, not a simplifier change

`docs/MIGRATION.md:7-20` labels nearly every port as pending, and `docs/MIGRATION.md:24` lists the old Artifact package, importer, BuildKit publisher, Node package, and process calls that still require removal. The reviewed workspace contains only the standalone service and does not establish that all callers use its protocol or that Zig Artifact/importer/build requirements are gone. The smallest correct next step is caller-by-caller migration evidence and a negative dependency/build check; preserving a compatibility bridge would violate the fixed cutover requirement.

### S3 — Operation history is broader than required and weaker than the claimed idempotency guarantee

**Priority:** high  **Confidence:** high  **Disposition:** ACCEPTED

Every request, including reads and `OPEN_*`, is inserted or checked in `operations` (`src/api/dispatch.rs:14-40`). Completed rows are pruned after roughly 4096 sequence positions (`src/api/dispatch.rs:37-39`), and the comment explicitly says IDs older than the window may be re-executed. A retry of an old `PIN`, `LEASE_CREATE`, `PREPARE`, or future `PUSH` can therefore execute again after a successful but unobserved completion. The table is also unnecessary for read-only requests and stores serialized results for operations that can derive their result from durable state.

Simplest correction: classify actions as read, idempotent mutation, or effectful transfer; persist intent/completion only for mutations whose retry identity must survive the caller's retry horizon. Keep a compact durable mutation record keyed by `OperationId`, or prove and document a bounded retention contract at every caller. Do not remove operation state until a test covers timeout-after-commit and replay after history pruning.

### S4 — Three state columns are currently dead or derivable state

**Priority:** medium  **Confidence:** high  **Disposition:** ACCEPTED as unnecessary complexity, pending schema migration

`roots.kind` and `roots.platform` are written in `src/oci/mod.rs:31-35` but never read anywhere in the service (`rg` finds only the writes and schema). The platform value is the request platform for every image in the graph, so it is not even a reliable per-root fact. `prepared.platform` is written in `src/prepare/mod.rs:29` and never read; the prepared ID already hashes the format, source manifest, and platform (`src/prepare/mod.rs:18-19`). Remove these columns in the next schema version. Retain the `roots` row itself because `verify_graph_if_known` uses root presence as the admission marker, and retain `prepared.size` because quota accounting sums it.

### S5 — Full test suite is red because the property fixture deletes its store

**Priority:** medium  **Confidence:** high  **Disposition:** ACCEPTED as test defect; not evidence against CAS integrity

`tests/cas.rs:70-79` binds the fixture as `let (_,mut store) = fixture()`. The returned `TempDir` is dropped immediately, while `Store` still references the deleted path. The full command `cargo test --manifest-path apollo-artifactd/Cargo.toml` failed in `stored_identity_matches_bytes` with `No such file or directory`; the proptest then produced a very large failure trace. The four other CAS tests pass when the property test is filtered out. Keep the property test, but bind and retain the `TempDir` for the test iteration. A green run after that fix is required; it does not establish the 100,000-operation churn gate.

## Simplifications that preserve required guarantees

1. Keep the current synchronous single-request server until throughput evidence demands concurrency. It has one bounded connection-processing path and avoids introducing task queues, cancellation state, or single-flight bookkeeping prematurely. Add bounded concurrency only with measured p95/p99 and FD/RSS evidence showing the single path is insufficient.

2. Keep the two-pass layer preparation. The first pass verifies the complete decompressed stream and DiffID before applying effects; the second pass applies the verified stream. Combining these passes without an equivalent durable spool would allow partial layer effects before a DiffID failure and would violate the safety requirement.

3. Keep `imports`, `gc`, and prepared phase state. They are recovery intents spanning filesystem effects and SQLite commits, so deleting them would remove crash recovery rather than simplify it. Their records should remain bounded by reconciliation/GC limits and must be covered by SIGKILL tests.

4. Keep graph edges and recursive reachability. Pin and lease protection must cover the complete OCI graph; replacing this with only the root digest would make layers/configs collectible. The graph can be re-derived for verification, but a durable edge set is currently the simplest bounded GC input.

5. Keep the generic protocol identities and FD transport. They satisfy the standalone boundary and prevent arbitrary filesystem paths crossing the API. Do not replace them with Apollo business IDs or path-based handles.

## Traceability and verification gaps

| Requirement / invariant | Current evidence | Status | Smallest closing evidence |
| --- | --- | --- | --- |
| Streaming CAS digest/size, private temp, sync, dedup | `src/cas/import.rs:10-61`; CAS tests | partial | concurrent import and crash-after-link tests |
| OCI index/manifest/config/layer verification | `src/oci/mod.rs:53-115`; `tests/oci.rs` | partial | malformed graph corpus and native Linux qualification |
| DiffID, whiteouts, safe rootfs | `src/prepare/layers.rs:31-110`; `tests/oci.rs` | partial | hardlink/device/symlink race and crash tests |
| Registry pull/push/auth/retry | explicit rejection in `src/api/dispatch.rs:74-76` | missing | bounded HTTPS client plus registry integration matrix |
| Pin/lease graph GC | `src/cas/references.rs:6-50` | partial | concurrent pin/release/GC and logical-corruption fail-closed tests |
| Crash-safe operations | `src/api/dispatch.rs:22-40` | partial | timeout-after-commit replay after retention boundary |
| Complete Rust cutover and no Zig dependency | migration map says pending (`docs/MIGRATION.md:22-28`) | missing | all-caller inventory, `cargo tree`, and production build negative checks |
| 100k/20k churn and performance measurements | no benchmark/churn artifact in `apollo-artifactd` | missing | native x86 and arm reports with RSS/CPU/threads/FD/p50/p95/p99 |
| Dependency audit and SBOM | `Cargo.lock`, `deny.toml` exist | unverified | `cargo audit`, `cargo deny`, `cargo tree`, SBOM output |

## Commands and evidence

- `cargo test --manifest-path apollo-artifactd/Cargo.toml`: failed only at `stored_identity_matches_bytes` because the test fixture dropped its temporary directory before importing.
- `cargo test --manifest-path apollo-artifactd/Cargo.toml --test cas -- --skip stored_identity_matches_bytes`: 4 CAS tests passed; the property test was intentionally filtered only to isolate the fixture defect.
- `rg` over `apollo-artifactd`: confirmed `roots.kind`, `roots.platform`, and `prepared.platform` have no read sites; confirmed `PULL`/`PUSH` are explicit rejects.

## Round state

This is one independent Simplifier round. No Red Team convergence claim is made, no production approval is made, and no code was changed. The next pass must re-evaluate the complete implementation after the registry/cutover plan, operation-retention decision, schema simplification, and fixture correction are concrete.
