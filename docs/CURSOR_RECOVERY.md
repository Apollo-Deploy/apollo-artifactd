# Incremental cursor repairs

The previous blob GC always inspected the first bounded page. A pinned prefix
could therefore permanently starve collectible later blobs. The repair persists
the scan cursor in the same transaction as deletion intents. The public Store
regression imports two independently hashed blobs, pins the first in digest
order, performs a one-entry GC, restarts, and proves the later blob is collected
while the pin remains readable. `gc-cursor-pre-fix.txt` records the intended
failure (zero collected instead of one); `gc-cursor-post-fix.txt` passes.

Prepared recovery had the same first-page starvation problem. Recovery and
prepared GC now have separate durable cursors. Preparing an existing incomplete
identity recovers that identity directly, avoiding recursive global scans. GC
directly completes the prepared deletion intents it selects. A recovery page
still verifies completed trees, but no longer rewrites already-complete records.

The recovery regression uses 4,096 real read-only complete directories and a
later pending record. The old scan retains 4,097 records; the repaired scan
retains the 4,096 complete directories and removes the pending record. Actual
pre/post outputs are `prepared-cursor-{pre,post}-fix.txt`.

These tests protect observable collection and crash-recovery progress. Existing
small-fixture pin/recovery tests could not expose page starvation. They use the
production Store/State boundaries and add no production test-only export or
fault flag. State fixtures model persisted crash records; they are not a
replacement for SIGKILL/power-loss qualification.

Current native CAS, OCI, recovery, API and protocol tests pass on both hosts in
`cursor-native-{arm64,x86_64}-tests.txt`; warning-free clippy outputs are
`redb-final-{arm64,x86_64}-clippy.txt`. Native source/debug binary identities are
recorded in `redb-registry-{arm64,x86_64}-snapshot.json`.

These repairs bound the candidate page and ensure progress. Full live-root
verification and per-candidate reachability still perform global work; this is
not approval of the required bounded incremental GC release gate.
