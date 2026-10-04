# Apollo Artifactd red-team round 4

Date: 2026-10-04
Verdict: **APOLLO_ARTIFACTD_PRODUCTION_BLOCKED**

This is an independent source review of the current dirty worktree. It did not
modify production code. The release gate remains blocked: the defects and
qualification gaps below are still present, and the full user-requested native
fault, registry, API-churn, and fuzz evidence is not available in this round.

## Mandatory findings

### RT4-001 — operation journal permanently exhausts at 4,096 IDs (HIGH)

`src/api/dispatch.rs:91-105` rejects a new operation when
`db.count("operations") >= 4096`. Completion and failure records are written at
lines 110-123, but there is no deletion, retention cursor, or sequence update;
every record is initialized with `sequence: 0`. The README describes an
approximately 4,096-record history, but the implementation has no pruning
policy.

Public-boundary reproduction on a qualified Linux daemon is a loop issuing 4,097
distinct mutation `operation_id` values (for example, bounded `STATUS`-adjacent
mutations using the protocol client). The first 4,096 can complete; the next
new ID returns `operation journal capacity exhausted` permanently until state is
manually altered. This is an attacker-controlled denial of service and fails
the required 100,000-operation churn gate. A retry with an old ID can still
replay its retained row, so this is also a long-term idempotency/history failure.

Required closure: bounded, crash-safe pruning or archival with a durable replay
contract, plus a public API churn test exceeding 100,000 unique operations and
a restart test proving the documented retention behavior.

### RT4-002 — prepared recovery only examined the first page (RESOLVED for the reviewed regression)

The current implementation persists separate prepared-recovery and prepared-GC
cursors (`src/prepare/recovery.rs:6-110`), recovers an incomplete identity
directly, and does not rewrite already-complete records. `docs/CURSOR_RECOVERY.md`
records the public Store regression with 4,096 complete directories followed by
a pending record; the repaired native reruns are recorded in
`docs/cursor-native-{arm64,x86_64}-tests.txt`. This closes the first-page
starvation regression within the tested bounded-page contract. It does not
close the separate global-work GC gate described below.

### RT4-003 — registry transfer qualification was previously ignored (RESOLVED for the recorded native scope)

The test remains opt-in because it requires an isolated authenticated registry,
but the explicit native runs in `docs/registry-final-arm64-tests.txt` and
`docs/registry-final-x86_64-tests.txt` pass both the core HTTPS transfer test
and the real daemon Unix-FD registry test. Those runs cover layered push/pull,
duplicate transfers, descriptor integrity, preparation, wrong references,
authentication failures, secret-free journals/errors, and canonical prepared
facts. The vendor upload-`Location`/digest fallback has a recorded pre/post
proof. This resolves the previously absent evidence for that scope only;
registry fault injection, fuzzing, and performance remain open release gates.

### RT4-004 — tag publication had no final race check (RESOLVED for the recorded final-tag observation)

`src/registry/push.rs:47-63` now performs an authenticated post-publication
read of the requested tag and rejects a changed tag. The final native registry
runs above exercise the resulting tag observation. A true concurrent-writer
stress campaign is still outside this resolved scope.

### RT4-005 — persisted OCI-edge verification ignores parents outside the rebuilt graph (LOW integrity gap)

`src/oci/mod.rs:107-128` reconstructs `graph` from the requested root and
compares persisted edges only for `graph.nodes`. A corrupt `edges` row whose
parent is not reachable from that root is silently omitted from `persisted` and
cannot make `verify_graph_if_known` fail. `DOCTOR` calls
`verify_live_graphs`, so it likewise does not detect this class of stale or
foreign edge. This leaves unowned reachability metadata in the database and
weakens state-corruption detection, but the current code does **not** make this
an observed content-safety bypass: pins, leases, and prepared roots begin
reachability from their recorded root; a detached edge parent is never reached,
and `recover_gc` removes the deleted child from every edge row while scanning
the full edge table. The demonstrated impact is stale metadata/diagnostic
availability rather than keeping arbitrary content alive or deleting protected
content.

Required closure: make the invariant explicit that every edge parent is a known
OCI root/manifest and every child is an admitted blob, then validate it in
bounded pages. A full scan belongs in `DOCTOR`/reconcile with a persisted cursor
or completion status; GC need only validate the bounded candidate and clean the
deleted child from all edge rows, as it already does. Add a public
doctor/restart corruption reproduction. The recent lost-root and foreign-sidecar
regressions are useful but do not cover this edge-table case.

### RT4-006 — DOCTOR reports its integrity limitation but does not perform the requested database check (MEDIUM)

`src/cas/mod.rs:149-154` runs live-graph checks and directory sync, then
returns `"database_integrity_checked": false`. This is honest and avoids a
false positive, but it means `DOCTOR` does not validate redb records, schema
decoding across tables, CRC/readability, ownership sidecars, or state/filesystem
consistency. The operation is therefore incomplete for the requested state
corruption and recovery gate. The redb descriptor-open/NOFOLLOW path itself is
a positive fix for the earlier SQLite pathname-reopen issue; it does not replace
an integrity doctor.

Required closure: implement a bounded, read-only redb/table/record/ownership
consistency check with explicit incomplete/busy status, and qualify corrupt
record, truncated DB, foreign sidecar, and restart cases.

## Positive disposition

The following earlier concerns appear improved in source and are not counted as
new failures here: redb is opened through an `O_NOFOLLOW` descriptor and passed
to `Builder::create_file` (`src/state/redb.rs:84-101`); unknown-digest imports
stream into private temporary storage before admission; reference and edge
scans in `src/cas/references.rs` paginate; registry error text is redacted in
`src/registry/mod.rs:56-59`; credential input is through a protected FD; and
the vendored client contains host-boundary and HTTPS-downgrade redirect checks.
These are source-level dispositions, not substitutes for native fault and
authenticated-registry evidence.

No approval is issued. The 4,096 operation exhaustion, global GC work,
incomplete database doctor, remaining caller Zig dependencies, and missing fault,
API-churn, and performance campaigns keep the production gate closed. The
native registry and cursor evidence above resolves only those specific prior
evidence gaps.

## Review identity and exact current hashes

The directory was dirty and uncommitted at review time. `HEAD` was
`3a421411cc1611f200b9f8e43ac29ef42c09a88d`; the following SHA-256 values are
the files reviewed in this worktree:

```text
Cargo.toml 47ba6b8a0e912bf5da7478cae7bb81e6ae678f8c939c5320b7b1f2ccaab498cf
Cargo.lock f7e6b65501704e3123500d39df17c0adf9038f158fd3653e50607a2c3a91bb68
README.md 6fe1b29a58ca91ca47bd8ce2662e734999e106babd5e5e1dec77da51618a942d
src/api/dispatch.rs 3ebe5317879139da5f32e012a72acc9feccf6a35f6bc78e0788b1f85a4c571df
src/state/redb.rs 915782b4cb530c13afea4a31b7aa2353606c55748ff81453e5b1532a34102e9a
src/registry/mod.rs 9dd29186d7c4bed06ca1aa001edff765a15c5474cf403e1aa7f16cd04226a35e
src/registry/push.rs c2ebac47e31388f4c19acf55186682cbc4a28126af9da84f1de8bcd6afa99396
src/registry/credentials.rs 3a68bfafd4db4e9e7274f74177584b09a2cab22387fdc59c7ac4c49aa1a5e921
src/cas/references.rs 3e8d3e4a12314bd7a4c6479291e3937565026e1e71710e1b461f0f0dbf7d4b3b
src/prepare/recovery.rs 8e63c1c0640e8fba3c5570c0fe0467f563657a3f0a5786d66c19074b1d312152
src/cas/recovery.rs be4d45f4dec7b2ed10c64996cc93548f7120860f719b95c06dd88223504ef020
src/oci/mod.rs edfb35bb740481a749ee18c7891a313edb80e7caffe9b64edfc08eea1a8580bd
tests/registry.rs 031071d721346cac11dfdc3fbe3bfdbb97c76eb8f9b7763f3f9b3199a9b14089
vendor/oci-client/src/client/support.rs 72293a93d171a75d0e419b714a11c4451b4887aa8d6de9685d858c183d944fed
```
