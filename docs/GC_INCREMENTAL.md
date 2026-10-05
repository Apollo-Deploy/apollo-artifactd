# Incremental GC marking — 2026-10-04

The collector and restart reconciliation share a durable mark cycle. A call
verifies at most `min(max, 64)` root records; it does not scan the entire pin,
lease or prepared inventory before returning. Each root graph still requires
bounded descriptor parsing and metadata verification, so a root-count bound is
not a constant-time or latency guarantee. Layer files get descriptor-relative
ownership and size checks during marking; their full content hashes remain
checked on reads and explicit verification rather than on each GC pass.

The phases are clear old marks, pins, leases, prepared records, then ready.
Ordered database cursors persist progress. Cursor-range exhaustion determines
phase completion, including when ordinary preparation recovery shrinks the
inventory. Marks and the root cursor commit together, preventing replay from
double-counting prepared protection. No blob or prepared deletion begins until
all roots have been verified in one unchanged reference epoch.

Each `gc_ready` call also shares an 8 MiB metadata IO budget (hash plus parse
read) and 512 graph-work units across all roots. An OCI root is expanded one
metadata document at a time; descriptor checks, persisted-edge comparisons,
and manifest classification each consume work units. The compact walk and its
cursor are checkpointed in the transactional state database. A partial walk
publishes no marks, so GC cannot sweep while only part of a pinned graph has
been proved. Restart discards that walk and safely starts the mark cycle again.

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

On 2026-10-05 a regression was added for a size-matching corrupted live layer:
GC must preserve it without rehashing the layer, while collecting unrelated
content. GC now reconstructs the graph from verified OCI metadata and requires
exact agreement with persisted edges and root classifications before marking.
This also rejects missing, extra, or substituted edges. The focused
`gc_incremental` and `oci` targets passed locally (3 and 9 tests) and on native
ARM64 through `ssh apollo-node-01-internet` (3 and 9 tests). The ARM checkout
used `/home/apollo-admin/artifactd-node-native-new/apollo-artifactd`; source
hashes for `src/oci/mod.rs`, `src/oci/reachability.rs`, and
`tests/gc_incremental.rs` matched this checkout. These targeted runs do not
establish full qualification or close the broader native release gates.

On 2026-10-05 the walk was changed from root-count-only accounting to the
per-call IO/work budgets above. A regression constructs a pinned two-platform
OCI index with two large manifests whose aggregate verification exceeds one
call's metadata budget. It confirms unrelated garbage remains before proof
completion, drops and reopens the store, then confirms collection completes
while both manifests, configs, and the shared layer remain. The existing CAS
cursor regression also exposed a phase-transition edge: finishing the last
root exactly at the root budget must still check whether its phase is exhausted.
GC now checks for another root before returning at the bound, preserving the
bound while allowing empty phases to transition.

The focused GC/OCI targets, the existing CAS cursor regression, and
`cargo test --locked --workspace` passed on both native x86_64 (`tihan-apollo`)
and ARM64 (`apollo-node-01-internet`) on 2026-10-05. The x86_64 full workspace
run used `CARGO_PROFILE_TEST_DEBUG=0` because the default test build exhausted
the host's tmpfs; the test profile change only omits debug symbols. The suite
reported root-only peer-identity tests, authenticated-registry tests, native
SIGKILL cases, and the 100k journal churn campaign as ignored, so those
environment-specific qualification gates remain open.

[Independent review](RED_TEAM_GC_INCREMENTAL.md) records the prepared-intent
repair and the remaining adversarial qualification requirements. Full churn,
performance, race/fault coverage and structural database Doctor qualification
remain release gates. This change does not authorize Zig removal or production
cutover.
