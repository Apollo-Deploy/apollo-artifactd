# Database integrity and Doctor maintenance — 2026-10-04

`DOCTOR` uses redb's full page/checksum/allocator integrity checker under the
store's exclusive ownership and the daemon's serialized request loop. It is
an expensive maintenance operation, not a constant-time liveness probe. The
library reloads bytes from disk rather than trusting its page cache. Its
mechanism can repair state, so Doctor requires a daemon-issued mutation token
and a durable intent before invocation.

A clean check followed by directory fsync restores access to the state. A
repair or any check/fsync error leaves that State instance quarantined. All
public reads and transactions reject further access. The service does not
write a completion into a database whose roots may have changed: it returns
an uncertain outcome and exits after sending the response. A damaged database
may refuse reopen; restart is not a promise that corrupted state is repaired
or that lost data has been restored.

State open also performs the full check before Apollo schema initialization
and artifact reconciliation. Embedded redb open/recovery precedes that check;
this is mature library recovery, not an Apollo reimplementation. Startup and
Doctor reject a non-clean result from the explicit checker. Qualification of
all corruption and power-loss classes remains required.

Both schema and ordinary state commits use `Durability::Immediate` and
redb's two-phase commit mechanism. Redb documents its stronger crash/checksum
semantics and also warns that durability still depends on storage honoring
fsync. No durability setting is reduced for qualification benchmarks.

Doctor's successful facts include `inspection_scope: operation_execution`.
A retry with the same token retrieves the historical inspection receipt and
does not repeat a possible repair. Allocate a new token for a current
inspection; artifactctl does this by default. `STATUS` remains an observation.
Journal inspection distinguishes currently pending intents from interrupted
intents converted to uncertain failures during recovery.

## Tests and evidence

The existing daemon API test is the healthy inspection/replay owner. Before
the repair it failed on `database_integrity_checked: false` on both native
hosts (`doctor-pre-fix-{arm64,x86_64}.txt`). It now requires page verification,
clean/non-repaired facts and an explicitly historical receipt on retry.

`tests/doctor.rs` writes and reads a uniquely committed marker, flips an
interior byte in the known live database page without changing file length,
and exercises both reopen and an active owner's uncached integrity check.
The active owner must reject subsequent mutation after the non-clean result.
A separate real daemon case corrupts the committed page after token
allocation, requires an explicit uncertain quarantine response, waits for
actual service termination and rejects follow-up connections. It uses no
production injection hook. If the damaged state is reopenable, the Doctor
operation must have an unresolved intent rather than an invented completion.

`doctor-native-{arm64,x86_64}.txt` records the final exact daemon corruption
check, normal all-target workspace suite, warning-free clippy and both real
SIGKILL import/prepared-GC tests with two-phase commits enabled. Both command
chains exited successfully. The matching debug build snapshots contain 79
source/qualification hashes checked against the local files. Earlier focused
logs remain separate; they do not establish the final assertion contract.

These contracts need separate boundaries: healthy checks do not reach page
corruption, and direct State quarantine does not prove socket response or
service shutdown. The fixtures exercise selected checksum damage, not all
semantic corruption, allocator corruption, malicious same-UID modification,
I/O failure or power-loss cases. The release remains unqualified until those
mandatory campaigns and the full cutover gates pass.
