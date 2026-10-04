# Red-team review: incremental GC and prepared recovery

**Date:** 2026-10-04  
**Scope:** incremental mark epochs, persisted cursors, prepared-output GC, and
restart recovery  
**Verdict:** `APOLLO_ARTIFACTD_PRODUCTION_BLOCKED`

This review is read-only and does not approve the implementation.

## Previously high finding: prepared GC intent recovery

**Previous severity: HIGH — availability and crash-recovery correctness.**
**Current disposition: fixed in source; native full qualification remains
pending.**

The previous implementation wrote the durable transition to
`phase = "gc_intent"` immediately before prepared-tree deletion (the current
transition remains at `src/prepare/recovery.rs:104-109`).
If the process is killed after that transaction commits and before
`recover_prepared_id` removes the record, restart executes `Store::open`'s
`gc_reset` and then `reconcile` (`src/cas/mod.rs:80-83`). The old mark cycle
treated `Prepared` rows whose phase was exactly `"complete"` as roots.
Consequently the surviving `gc_intent` row received no prepared mark.

Recovery then reaches `recover_prepared_record` (`src/prepare/recovery.rs:49-81`),
passes the ready-snapshot check, and calls `gc_complete_prepared`. That method
requires an epoch-matching mark with `prepared > 0`; because the intent was
excluded from the rebuilt Prepared phase, it fails with “missing prepared GC
protection”. The row and possibly its already-deleted tree remain wedged. A
subsequent restart rebuilds the same mark set and repeats the failure. This is
the exact crash window the durable intent is meant to close.

The normal recovery tests cover manually inserted publication/GC records. A
real daemon SIGKILL fixture now covers one observed window between durable
`gc_intent` publication and partial prepared-tree deletion; complete
effect-boundary coverage is still a separate gate.

The repair includes `gc_intent`/`deleting` records in the rebuilt Prepared mark
phase, and `prepared_dirty` distinguishes a GC-owned
`complete -> gc_intent` transition from a new protection mutation. The
persisted prepared count and cursor therefore allow `gc_complete_prepared` to
decrement the matching mark and remove the record. The targeted pre-fix and
post-fix records (`docs/gc-prepared-intent-{pre,post}-fix.txt`) demonstrate the
former “missing prepared GC protection” failure and the repaired behavior.
Those records use an actual core recovery regression, but are not a SIGKILL
campaign at each filesystem effect boundary; native full proof of those
windows remains pending.

## Mark-epoch and cursor observations

The epoch invalidation policy is directionally sound: pins, leases, roots,
edges, and publication of a complete prepared output dirty the transaction and
advance `gc_reference_epoch`; the cycle and marks are committed together.
Restart clears the cycle and boundedly clears persisted marks before rebuilding
it. The targeted post-fix regression covers the prepared-intent state, while
native SIGKILL coverage at each filesystem effect boundary remains absent.

The current `max.min(64)` root budget bounds one mark invocation, while the
persisted phase/cursor bounds progress across calls. The new cursor EOF probe
avoids the former global-count/suffix-skip condition when ordinary prepared
recovery deletes rows. This does not yet prove bounded total reconciliation
under churn: protected-root graph walks remain per-root work, and
mutation/crash tests must show that a continuously changing prepared
inventory cannot starve old cursor positions or repeatedly restart a cycle
without a bounded recovery outcome.

## Integrity/ownership disposition

`verified_graph_nodes` recomputes OCI relationships from descriptor bytes and
compares persisted edges, so an extra foreign edge or digest alias fails graph
verification rather than becoming GC authority. A prepared mark is counted per
prepared record over the deduplicated graph node set; shared nodes are
protected while any handle or prepared reference remains. This review found no
new evidence that a shared reachable node is under-counted in the ordinary
single-owner path.

No additional high finding was established in this read-only pass. The release
gate remains blocked on native fault/race qualification for the repaired
windows, globally bounded GC work, structural database Doctor checks, and the
remaining production cutover requirements. No production approval is
inferred.

## Review of `tests/gc_crash.rs`

The ignored Linux fixture creates a large real prepared tree, sends `GC`
through the daemon socket, waits until the tree has partially disappeared,
kills the daemon process, and checks the durable prepared intent before
restart. It now replays the interrupted `GC` with the same `OperationId`,
expects the documented `outcome uncertain` error, verifies the failed terminal
journal result, preserves the manifest, removes the prepared record, and
asserts that the entire prepared inventory (including staging entries) is
empty. This covers the prior journal/replay and staging assertions gap.

`docs/gc-sigkill-arm64.txt` records one passing ARM64 run (23.95 seconds), and
`docs/gc-sigkill-x86_64.txt` records one passing x86_64 run (50.91 seconds).
The initial x86_64 attempt failed only in the daemon readiness helper while
large-tree startup recovery exceeded its old timeout; the bounded readiness
timeout was then increased. Each native run observes one partial-deletion
window and does not establish coverage for every filesystem effect boundary,
repeated crash point, global-GC churn, or release qualification.
