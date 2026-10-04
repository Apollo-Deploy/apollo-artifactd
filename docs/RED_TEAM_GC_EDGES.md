# Red-team review: GC descriptor edges

**Date:** 2026-10-04
**Scope:** `src/cas/recovery.rs`, `tests/oci_gc.rs`, and the GC edge
documentation
**Verdict:** `APOLLO_ARTIFACTD_PRODUCTION_BLOCKED`

This is a targeted integrity review of the repair that stopped `recover_gc`
from rewriting every surviving parent edge when one CAS object is collected.
It is not a release approval.

## Disposition of the repair

The repair is correct for the demonstrated surviving-parent/collected-leaf
case. GC removes the collected object's outgoing edge row and its own
`roots`, `blobs`, and `gc` records, while retaining descriptor edges owned by
surviving parents. A surviving manifest whose config or layer is absent is an
incomplete cache graph: `pin` and `resolve` fail closed. Reimporting the exact
leaf bytes under the same digest restores the retained relationship, including
after reopening the store. A different byte sequence cannot satisfy the
descriptor digest.

The behavior is intentionally different when the collected object is itself
an OCI manifest or index. Its outgoing relationships and root admission are
removed with that object. Raw `IMPORT_BLOB` of the same JSON bytes does not
re-admit those relationships; restoration requires `IMPORT_OCI`/OCI graph
admission. The test and documentation must not generalize the leaf reimport
result to deleted intermediate graph nodes.

Unpinned incomplete graphs are valid cache state. Their dangling descriptor
edges are metadata only: they do not authorize opening or exposing a child,
and they do not authorize deleting a protected object. A protected manifest or
index must have a complete, verified reachable graph, and every reachable
child remains protected. Once the parent itself is collected, its owned edge
row is removed. This is the refined form of the earlier Round 4 “every child
must be admitted” invariant; requiring every cached edge to have a live child
would reject the intended incomplete-cache behavior.

## Evidence

`tests/oci_gc.rs` uses public `Store` operations. It admits a zero-layer OCI
manifest, collects the config with `GC(1)`, checks that the surviving manifest
cannot be pinned or resolved, reimports the exact config bytes, reopens the
store, and verifies that pinning succeeds and protects both objects. No
fault-only flag or test-only production export is used.

The pre-fix records
`oci-gc-edge-pre-fix-{arm64,x86_64}.txt` reproduce the post-restart
reachability failure. The latest native records
`oci-gc-edge-final-{arm64,x86_64}.txt` show the regression passing on ARM64 and
x86_64, alongside the normal workspace tests and clippy. The registry tests
shown in those logs remain ignored where an isolated authenticated HTTPS
registry is required; that does not affect this local edge regression.

The wording in `docs/GC_DESCRIPTOR_EDGES.md` now explicitly labels the
parent-edge removal as the pre-fix regression. The current repair and passing
regression deliberately preserve that incoming parent edge, so the report
does not treat the corrected sentence as a current-invariant defect.

## Release-blocking gaps

1. **GC is not globally bounded.** `recover_gc` calls `verify_live_graphs`, and
   candidate protection calls can scan all handles and reachable references.
   The `max` candidate bound therefore does not bound total work. Persisted
   incremental cursors, protection against reference mutations between pages,
   and native crash/race/resource qualification are still required.
2. **Structural database health is unqualified.** `DOCTOR` reports
   `database_integrity_checked: false`; live graph verification is not a
   redb structural corruption check. Recovery, torn/corrupt records, and
   ownership metadata need an explicit bounded structural audit.
3. **Fault and race coverage remains incomplete.** This focused test does not
   qualify SIGKILL during the repaired edge transaction, concurrent pin or
   lease admission against GC, malformed/stale edge records, or foreign-file
   ownership races.

Consequently the repair closes the specific stale-parent-edge regression, but
the required global bounded-GC and structural-recovery gates remain open and
the artifactd release must remain blocked.
