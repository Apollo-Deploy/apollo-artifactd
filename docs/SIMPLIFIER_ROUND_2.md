# Simplifier review, round 2

**Mode:** implementation review, independent simplifier pass

**Baseline:** current standalone `apollo-artifactd` workspace after the round-1 corrections. No production code, tests, fixtures, state, or deployment artifacts were changed by this pass. Registry and caller cutover gates remain outside this round and are still incomplete.

**Verdict:** `Block release`. The property fixture now passes, duplicate imports bypass quota accounting when the digest is already present, graph corruption is checked before GC, and SIGKILL recovery exists as a test. Native Linux qualification, registry implementation, caller migration, and the required release gates remain absent. One redundant integrity read and one unbounded GC verification path remain.

## Findings

### S2-1 — Descriptor validation hashes every descriptor twice

**Priority:** medium  **Confidence:** high  **Disposition:** ACCEPTED as unnecessary cost

`src/oci/mod.rs:226-239` calls `verify_file(&digest, descriptor.size())`, which opens, size-checks, streams, hashes, rewinds, and returns a verified file. It then calls `open_blob(&digest)` at lines 233-237, which performs the same database lookup, metadata check, full SHA-256 pass, and rewind again. The second handle is only used for a second length check that `verify_file` already established. OCI import, resolution, and every GC live-graph verification therefore pay two full reads per descriptor.

Remove the second `open_blob` and use the successful `verify_file` result (or change `verify_file` to return only a verified size marker). Preserve the first full verification; removing both would weaken the CAS invariant. Add a test or instrumentation check that descriptor validation performs one content hash per descriptor.

### S2-2 — GC `max_entries` does not bound live-graph verification work

**Priority:** medium  **Confidence:** high  **Disposition:** ACCEPTED as a bounded-work gap

`src/cas/references.rs:70-81` verifies every pin, lease, and prepared root before each GC call, and each root rebuilds and rehashes its complete OCI graph. `gc(max)` only limits candidate rows and recovery rows (`:85-106`); a store with 100,000 references can force an unbounded full scan and repeated descriptor hashing even when `max_entries` is 1. The comments acknowledge that this needs incremental scheduling.

Do not delete graph verification: the corruption test shows it is a deliberate fail-closed invariant. Simplify the scheduling instead by persisting a bounded verification cursor/epoch, or by verifying one bounded root batch per reconciliation cycle and refusing deletion for unverified roots. The state must make unfinished verification explicit so SIGKILL cannot silently convert “not checked” into “safe to delete.”

### S2-3 — Operation history is now read-only-free, but the retention boundary still changes identity semantics

**Priority:** medium  **Confidence:** high  **Disposition:** ACCEPTED; narrowed from round 1

The correction at `src/api/dispatch.rs:41-55` correctly bypasses `operations` for observations and FD opens. Current mutations are individually idempotent in the reviewed paths: duplicate CAS imports verify input, references use conflict-safe inserts, preparation derives a stable ID, and GC uses intents. However, completed and failed operation rows are still pruned at `:97-100`, and the comment says old IDs may be re-executed. After pruning, the same `OperationId` no longer guarantees the prior response or request-conflict behavior. That becomes material when registry `PUSH`/`PULL` are added, because a timeout-after-remote-commit can replay an external effect.

Keep operation rows only for actions whose result or external effect cannot be reconstructed safely, and define the retention horizon as part of the protocol. For current local mutations, derive the result from durable state where possible. Before registry support, add a test that exercises replay after pruning; after registry support, retain a durable transfer identity until remote reconciliation proves completion.

### S2-4 — Schema simplification from round 1 remains unapplied

**Priority:** low  **Confidence:** high  **Disposition:** ACCEPTED as unnecessary persisted state

`state.rs:39` still defines `roots.kind` and `roots.platform`, and `state.rs:43` still defines `prepared.platform`. The service writes these fields but has no read sites. Prepared identity already includes format, source manifest, and platform (`src/prepare/mod.rs:18-19`), while root admission is determined by row presence and reconstructed OCI bytes. Remove the fields in the next schema version, retaining the rows, `prepared.size`, and phase/recovery fields that have active lifecycle roles. This is a cleanup item, not a release blocker by itself.

## Requested fix checks

### Property fixture

`tests/cas.rs:113-122` still binds the returned temporary directory as `_root`, but the current full CAS run passes all five tests, including `stored_identity_matches_bytes`. The store owns directory descriptors and uses descriptor-relative paths, so dropping the `TempDir` handle does not reproduce the prior failure on this host. Retaining the binding would make the test intent clearer and avoid relying on descriptor lifetime semantics, but the prior failure is no longer present in the observed run.

### Duplicate import at quota

`src/cas/import.rs:18-35` checks for an existing digest before quota calculation at `:37`. `tests/cas.rs:107-110` fills a four-byte store, repeats the same digest successfully, then rejects conflicting bytes. This is the correct ordering: deduplication does not consume quota, while conflicting input is still streamed and verified. The duplicate path performs a full verification of stored bytes and a full hash of input; that cost is justified by the CAS integrity requirement.

### GC graph verification

`src/oci/mod.rs:60-83` compares reconstructed graph edges against persisted edges, and `tests/oci.rs:111-128` deletes manifest edges and confirms GC fails while all three blobs remain. The check closes the tested missing-edge case. It does not yet bound the amount of verification work, as recorded in S2-2, and the test does not cover a stale edge whose parent is outside the reconstructed graph.

### Linux directory fsync

`src/filesystem.rs:87-95` now reopens `.` relative to the capability with ordinary read-only directory flags before `fsync`, addressing the earlier `O_PATH` concern in the code path. This was not executed on Linux in this pass: the local host is `Darwin 27.2.0 arm64`. `ssh tihan-apollo` confirmed `Linux 6.12.107+deb13-amd64`; `ssh apollo-node-01` confirmed `Linux 6.8.0-146-generic aarch64`, but the source was not built or qualified remotely. Native Linux fsync, crash, and ownership evidence therefore remains unverified.

### SIGKILL import recovery

`tests/recovery.rs:95-139` starts a child import worker, waits until a non-empty temp file exists, sends SIGKILL, and reopens the store. The test passes locally and confirms no final blob resolves, no partial temp remains, and the blob count is zero. It is a real process kill test, but it ran on macOS; it does not close the required native Linux x86/arm qualification gate. The test also covers interruption during temp writes, not crash-after-hard-link publication or crash during preparation/GC.

## Simplifications that should remain

- Keep operation rows for durable mutation intent and result until the protocol defines a safe derivation for each mutation. Removing the table now would discard the required operation tracking and recovery boundary.
- Keep `imports`, `gc`, and prepared phase rows. The recovery tests exercise their filesystem/database ordering; they are not speculative state.
- Keep graph edges and graph reconstruction. The corruption test demonstrates that trusting only root reachability would permit unsafe GC.
- Keep duplicate input hashing. A stored digest alone proves the existing object, not that the newly supplied stream matches the caller's declared identity.
- Keep two-pass layer extraction. It preserves the “no effects before DiffID verification” invariant; the extra read is the cost of that guarantee.

## Verification record

- `cargo test --manifest-path apollo-artifactd/Cargo.toml --test cas`: 5 passed, including the previously failing property test.
- `cargo test --manifest-path apollo-artifactd/Cargo.toml --test recovery`: 4 passed, including `sigkill_during_import_never_resolves_partial_content`.
- `cargo test --manifest-path apollo-artifactd/Cargo.toml --test oci`: 5 passed, including `corrupt_reachability_cannot_authorize_gc`.
- `cargo test --manifest-path apollo-artifactd/Cargo.toml --test api`: 0 tests executed on the local Darwin target because the API test is Linux-gated.
- `ssh tihan-apollo 'uname -srm'`: Linux x86_64 confirmed; `cargo` available.
- `ssh apollo-node-01 'uname -srm'`: Linux aarch64 confirmed; no `cargo` was reported.
- No remote source transfer, build, or test was performed. No registry/caller, churn, performance, audit, or release claim follows from these local tests.

## Round state

This is independent Simplifier round 2. The round did not produce release approval, convergence, or a clean two-round red-team result. Registry support, caller cutover, native Linux qualification on both requested hosts, bounded incremental GC verification, and the remaining crash cases must be addressed before another release review.
