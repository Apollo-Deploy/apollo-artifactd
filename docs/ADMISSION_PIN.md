# Atomic OCI admission and pinning

`IMPORT_OCI` and `IMPORT_OCI_ARCHIVE` optionally accept a generic `pin`.
Artifactd verifies the complete OCI graph and platform, checks pin identity
ownership and capacity inside the redb write transaction, and publishes graph
edges, root classification and the pin together. The response includes
`pin_id`. Same-pin/same-digest requests are idempotent; a conflicting digest
fails before graph rows are written. Verified CAS blobs already imported from
an archive may remain unreferenced after failure and can be collected safely.

The API mutation journal persists intent before effects and completion after
admission. A committed pin survives uncertain operation completion; restart
reconciliation does not discard pins. Callers must keep their chosen PinId in
their own durable lifecycle state and release it only when no longer needed.
Omitting the pin preserves the generic unprotected import contract. Registry
pull admission does not yet accept this optional pin and requires further work.

The test-audit owner is the real Unix-socket daemon test `admission_pin`: an
FD archive import, fresh-token direct admission with the same pin, conflict
against another graph, GC retention, daemon restart/token replay, GC retention
after restart, and unpin collection. There is no production test hook.

The controlled pre-fix ARM run removes only the transactional pin write while
preserving the response and protocol. It fails after successful GC when the
original artifact no longer verifies (`admission-pin-pre-fix-arm64.txt`). The
first attempted control used an invalid GC bound and is not regression proof;
that harness issue was corrected before the retained intended-failure run.

Native qualification and exact source snapshots are recorded separately for
ARM and x86. Normal workspace suites skip explicit ignored long registry/fault
campaigns; previous campaign evidence is not attributed to this new revision.
The scoped independent review is `RED_TEAM_ADMISSION_PIN.md`. Full caller
cutover, service-identity isolation, crash/fault qualification and old Zig
removal remain mandatory release gates.

Both native hosts passed the final focused daemon regression, normal locked
workspace/all-targets suite and clippy with warnings denied. Both also passed
release/all-targets builds of the production change. Final snapshot hashes
match all 99 source/build inputs on each host. The final test-only strengthening
followed those release builds; release production code did not change.

Commands: `cargo test --locked --test admission_pin`,
`cargo test --locked --workspace --all-targets`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`, and
`cargo build --locked --release --all-targets`. See `admission-pin-native-*`
for release evidence and `admission-pin-final-*` for final tests/snapshots.
