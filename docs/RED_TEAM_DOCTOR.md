# Red-team review: Doctor integrity quarantine

**Date:** 2026-10-04
**Scope:** `State::check_integrity`, redb quarantine guards, Doctor dispatch,
journal recovery, and startup behavior
**Verdict:** `APOLLO_ARTIFACTD_PRODUCTION_BLOCKED`

This is an independent read-only review. It does not approve the release.

## Disposition of the alleged Doctor bypass

The initial review incorrectly treated `Store::doctor`'s
`self.db.check_integrity()` at `src/cas/mod.rs:153` as a direct redb call. The
compiled type of `Store::db` is `state::State` (`src/cas/mod.rs:51`), and method
resolution invokes `State::check_integrity` from `src/state/integrity.rs`,
which marks the state unhealthy before the redb check, fsyncs repairs, and
restores health only for a clean result. The redb `Database` is private inside
`State`, so the reported HIGH bypass is a false positive and is withdrawn.

The remaining qualification requirement is real: API tests must exercise both
repair and error outcomes through this wrapper and prove that the quarantined
state blocks reads/transactions, prevents journal completion, returns an
uncertain result, and causes the daemon to require restart. Current direct
corruption coverage is useful but does not by itself prove every Unix API
branch.

## Disposition of Doctor replay semantics

Doctor is currently treated as a mutation, so `journal::begin` returns the
cached result for a completed `OperationId` before dispatch runs a fresh
integrity check. This deliberately returns a historical inspection receipt,
and `docs/DOCTOR_INTEGRITY.md` documents that callers must allocate a new token
for a current inspection. The behavior is therefore a documented protocol
choice rather than an unresolved defect; qualification should still ensure
artifactctl follows that token rule.

## Startup integrity policy

`State::open` now runs the explicit integrity check before schema
initialization, while embedded redb open/recovery necessarily precedes that
check. `Store::open` only reaches `gc_reset`/`reconcile` after a clean result.
`DOCTOR_INTEGRITY.md` documents this boundary and the limitation that mature
redb open/recovery is not an Apollo repair implementation. Qualification of
all corruption, allocator, I/O, and power-loss classes remains open, but no
ordering bypass was found in this review.

## Evidence and remaining gates

`tests/doctor.rs` now exercises direct State corruption, API uncertain
responses, retained intent, daemon termination, and blocked follow-up
connections. `tests/api.rs` covers clean Doctor facts and the documented
historical replay. Native focused results cover the selected corruption fixture
on both hosts; they do not cover every semantic, allocator, malicious same-UID,
I/O, or power-loss class.

No additional material Doctor defect was established in this pass. The release
remains blocked on broader structural corruption/fault campaigns and the
artifactd production gates. No release approval is inferred.
