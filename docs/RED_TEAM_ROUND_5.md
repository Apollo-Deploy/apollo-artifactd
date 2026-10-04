# Apollo Artifactd red-team round 5

Date: 2026-10-04
Verdict: **APOLLO_ARTIFACTD_PRODUCTION_BLOCKED**

This targeted review covers the protocol-v2 operation tokens, bounded journal,
startup recovery, and the latest native evidence. It is an independent
qualification review; it made no production source changes and does not grant
release approval.

## Prior high finding closed

### RT5-001 — permanent 4,096-operation exhaustion (CLOSED for availability)

The previous implementation rejected the 4,097th new operation permanently.
The recorded ARM pre-fix probe reproduces that exact failure in
`docs/journal-capacity-pre-fix-arm64.txt`.

The current daemon allocates canonical epoch/sequence tokens, persists a
retirement floor, and retires terminal results and unused reservations in
batches. The public daemon allocation/`UNPIN` churn campaign completes 100,000
operations on both native architectures while retaining 4,064 operation rows:

- `docs/journal-churn-arm64.txt`
- `docs/journal-churn-x86_64.txt`

This closes the permanent availability defect and demonstrates bounded journal
state at that workload. The runs are debug builds and are not release
performance evidence.

## Recovery disposition

Startup now audits all retained records in pages of 64 before binding the Unix
socket. It checks contiguous token sequence, canonical key/sequence mapping,
request/action and phase/result consistency, and exact registry companion
parity. Interrupted intents become terminal failures with the explicit
uncertain-outcome message; the old token is never re-executed. The audit also
runs from `DOCTOR` without mutating records. Allocation pressure therefore
cannot leave a leading unresolved intent permanently wedging the live daemon.

The native crash evidence now passes streamed import and interrupted registry
pull recovery on ARM and x86_64:

- `docs/journal-crash-arm64.txt`
- `docs/journal-final-x86_64-checks.txt` (registry SIGKILL case: 272.95s)

The tested registry case confirms no partial blob publication and preserves the
uncertain operation outcome. This is a strong public-boundary result using the
real daemon, Unix socket, SCM_RIGHTS, and isolated registry.

The actual effect-completion crash window remains only partially qualified.
The current test kills transfers during partial work; it does not prove a kill
after a remote registry has committed an effect but before artifactd commits
its terminal journal result. That case must continue to return uncertainty and
must not repeat the old token. The retained x86 artifact records two earlier
failed attempts: one ENOSPC failure and one 30-second socket timeout. The
protocol client now waits up to 600 seconds for a bounded operation response
while packet delivery remains bounded at 30 seconds (`wire::wait_for` and
`client::call`); the subsequent 272.95-second x86 rerun passed. The same final
artifact records the normal workspace suite and warning-free clippy pass. The
earlier failures remain part of the evidence history and show why the deadline
change required rerunning the campaign.

## Remaining mandatory findings

### RT5-002 — daemon journal churn is qualified only for the supplied campaign (MEDIUM gate)

The two 100,000-operation runs prove bounded public allocation and mutation
state for their `UNPIN` workload. They do not qualify mixed mutation classes,
concurrent clients, large result records, registry operation retention, or
crash/restart churn at that scale. Native test suites still mark the churn and
crash tests as opt-in; the retained native campaign artifacts are the evidence,
not ordinary `cargo test` coverage.

Required closure: run the documented 100,000 public-API campaign with mixed
local mutations and bounded restart/crash checkpoints, then repeat the relevant
registry cases with real remote-effect uncertainty.

### RT5-003 — legacy journal migration remains an operational gate (MEDIUM)

Nonempty version-1 operation journals are deliberately rejected rather than
silently converted. This is fail-closed and avoids replay ambiguity, but no
qualified migration procedure exists. Reusing an existing store containing
legacy operation rows therefore stops service startup until an explicit
migration is performed.

Required closure: ship and qualify an offline migration that preserves or
retires old IDs without allowing their effects to execute twice, or document a
supported store replacement and data-preservation procedure with native tests.

### RT5-004 — global GC work remains outside the bounded incremental contract (HIGH gate)

The cursor repairs prove candidate-page progress and prepared recovery progress,
but `GC` still performs global live-root verification and per-candidate
reachability work before bounded deletion. `docs/CURSOR_RECOVERY.md` explicitly
records this limitation. A `max_entries` request therefore does not bound total
GC work, latency, or database scans as required by the user contract.

Required closure: make live-root/reachability maintenance incremental with
durable progress and fail-closed incomplete-scan semantics, then qualify churn,
restart, and concurrent pin/lease cases.

### RT5-005 — DOCTOR still does not provide structural database corruption proof (MEDIUM gate)

The journal semantic audit now validates the retained operation records and
registry relationships. `Store::doctor` still returns
`database_integrity_checked: false`; it does not establish redb structural
readability/CRC health or full state/filesystem consistency. Journal semantic
verification must not be reported as a database integrity check.

Required closure: add a bounded read-only redb/table/record/ownership check with
explicit incomplete status and qualify corrupt-record, truncated-database,
foreign-sidecar, and restart cases.

### RT5-006 — complete cutover and release qualification remain open (HIGH gate)

The current evidence does not prove that all production callers use artifactd,
that the old Zig Artifact/importer/process paths are removed, or that there is
no Zig fallback. Full native fault campaigns, long native fuzz campaigns,
representative layered-image/API/registry performance, and release-build
performance evidence are also incomplete. These are mandatory completion-gate
items, not future enhancements.

## Evidence boundary

The latest journal implementation now has a coherent bounded-token contract:
retained-window replay returns the original terminal result, expired tokens are
rejected, request conflicts are rejected, interrupted intents become explicit
uncertain failures, and forged future/foreign-epoch tokens do not allocate
effects. The 100,000 public daemon churn artifacts materially close the former
4,096 permanent-exhaustion finding.

They do not establish `RED_TEAM_RELEASE_APPROVED` or
`APOLLO_ARTIFACTD_PRODUCTION_COMPLETE`. The remaining GC, database, migration,
cutover, crash-effect, and full qualification gates keep the release blocked.
