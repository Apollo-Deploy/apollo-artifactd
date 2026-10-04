# Incremental GC marking — 2026-10-04

The collector and restart reconciliation share a durable mark cycle. A call
verifies at most `min(max, 64)` root records; it does not scan the entire pin,
lease or prepared inventory before returning. Each root graph still requires
bounded descriptor parsing and verified blob reads, so a root-count bound is
not a constant-time or latency guarantee.

The phases are clear old marks, pins, leases, prepared records, then ready.
Ordered database cursors persist progress. Cursor-range exhaustion determines
phase completion, including when ordinary preparation recovery shrinks the
inventory. Marks and the root cursor commit together, preventing replay from
double-counting prepared protection. No blob or prepared deletion begins until
all roots have been verified in one unchanged reference epoch.

Schema 3 adds the mark table and reference epoch transactionally to schema 2.
Changed pins, leases, admitted roots/edges and complete prepared publications
advance the epoch in their transaction. A stale cycle restarts marking before
any sweep. Prepared GC records retain protection through `gc_intent` and
`deleting`; removal completion decrements their graph marks and removes the
record in one transaction. Restart discards the previous cycle and rebuilds
marks from surviving records, including interrupted prepared deletion intents.

Blob sweep and pending deletion recovery remain bounded by the requested
batch. Surviving parents retain immutable descriptor edges when a leaf is
collected. A completed blob pass resets marking so negative reference changes
cannot leave permanent protection. Calls may return zero while marking makes
progress. Continual reference changes can postpone collection; deletion fails
closed rather than trusting an incomplete snapshot.

## Evidence

`gc-incremental-pre-fix-{arm64,x86_64}.txt` captures the earlier global startup
walk failing immediately on a late corrupt pin. The repaired test opens the
store, advances bounded calls to that pin, rejects its missing content and
keeps both the pinned and collectible blobs intact. A second test mutates pins
and leases during ready and partial snapshots, checks safety throughout 128
calls, then checks eventual collection after release.

`gc-incremental-native-{arm64,x86_64}.txt` records both tests, the immutable
OCI edge regression, seven recovery tests and warning-free all-target Clippy
on native Linux. `gc-prepared-intent-pre-fix.txt` demonstrates the excluded
prepared-intent recovery failure; the post-fix log proves reconciliation both
before and after the tree effect. Those two states are constructed after
stopping the store; they do not establish actual SIGKILL effect-boundary proof.

`gc-sigkill-{arm64,x86_64}.txt` additionally records real daemon SIGKILL
after observing partial deletion of a 20,000-entry prepared tree. Restart
cleans the surviving intent, preserves the manifest, and rejects replay of
the interrupted GC token as an uncertain outcome. This is one observed effect
window per native host, not a complete crash-point campaign. The initial x86
readiness failure is retained separately: the test helper waited only 10
seconds for large-tree recovery. Extending that bounded wait to 60 seconds
allowed the native rerun to pass without changing production behavior.

`gc-incremental-final-{arm64,x86_64}.txt` and their source snapshots record the
normal workspace suites and warning-free Clippy after the final test changes.

[Independent review](RED_TEAM_GC_INCREMENTAL.md) records the prepared-intent
repair and the remaining adversarial qualification requirements. Full churn,
performance, race/fault coverage and structural database Doctor qualification
remain release gates. This change does not authorize Zig removal or production
cutover.
