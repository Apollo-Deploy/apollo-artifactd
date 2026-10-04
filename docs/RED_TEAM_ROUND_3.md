# Apollo Artifactd red-team round 3

Date: 2026-10-04

This pass reviewed the latest filesystem/state sidecar checks, OCI graph
classification fallback, foreign-sidecar regression, deleted-root/edge GC
regression, and the available qualification artifacts. No production code was
changed. The standalone directory is not a Git repository, so none of these
artifacts should be read as committed. Verdict: **BLOCK RELEASE**.

## Prior findings disposition

The following round-two findings are still open:

- **RT2-001 / RT2-003 (Critical/High):** `src/api/dispatch.rs:171-175` still
  advertises `registry:false` and rejects both `PULL` and `PUSH`. There is still
  no credential FD/provider contract, registry authentication, transfer retry,
  or registry fault matrix.
- **RT2-002 (High):** `docs/MIGRATION.md:16-19,24` still records direct
  publisher/BuildKit/Node integration and Zig importer/process removal as
  pending. No production caller cutover is demonstrated here.
- **RT2-004 (Medium):** `src/api/dispatch.rs:103-105` still prunes completed
  operation records after roughly 4096 sequence positions and explicitly
  permits old IDs to re-execute. The README documents this retention limit, but
  the requested durable long-term replay guarantee and a post-pruning replay
  test still do not exist.

The following evidence gaps are partly closed, but not fully:

- **RT2-005 (Medium):** prior native x86_64 and aarch64 test/build, 100,000
  blob and 20,000 OCI lifecycle churn, idle/resource, throughput, and
  dependency-audit artifacts exist in the standalone directory. They show
  bounded FDs and one thread, and include p50/p95/p99 churn latency. These are
  prior successful runs, before the latest primary-root fallback and
  foreign-sidecar changes; final reruns are still in progress. They do not
  cover registry, disk-full, crash-after-publication,
  crash-during-preparation, crash-during-GC, or authenticated transfer. Fuzz
  logs are macOS runs (`/Users/.../Library/Caches/.../aarch64-apple-darwin`),
  not native Linux qualification, and do not cover the missing registry parser.

## New finding

### RT3-001 — High — SQLite state is checked through one FD, then reopened by pathname

`src/state.rs:8-18` opens `state.sqlite` relative to the private root and
validates the returned inode. On Linux, however, `src/state.rs:20-22` then
constructs `/proc/self/fd/<root-fd>/state.sqlite` and `Connection::open` opens
the pathname again; the validated `state.sqlite` FD is discarded. Between the
first `openat` and SQLite's second open, same-UID code able to mutate the store
can replace or rename the file. The sidecar loop at lines 29-43 only validates
the three sidecars present at that instant and does not establish that the
database connection is the inode that was checked.

The README limits the trust model to cooperating processes running as the
service UID. Within that narrower model this is a documented boundary, but it
does not satisfy the user's requested full TOCTOU/foreign-mutation defenses.
This remains a concrete TOCTOU gap in the requested filesystem/state threat
model.
The new `foreign_database_sidecars_are_rejected_without_cleanup` test is good
for a symlinked WAL sidecar, but it cannot catch replacement of the primary
database in the open/reopen window. The daemon's owner lock prevents another
well-behaved artifactd, not hostile same-UID filesystem mutation.

Required closure: use a SQLite VFS/open path that consumes the already-validated
descriptor, or hold and compare inode/device metadata across the SQLite open
and reject any mismatch. Add a Linux race/fault test proving the connection
cannot attach to a replacement database and that foreign sidecars are never
removed.

## OCI fallback assessment

`src/oci/mod.rs:60-99` now treats a missing `roots` classification as an OCI
graph when a small blob parses as schema-version 2 metadata with `manifests` or
`config`/`layers`, then reconstructs and compares the complete edge set. This
closes the tested classification-row-lost path by failing closed rather than
silently treating a pinned graph as a raw blob. The deleted-edge and deleted
root tests in `tests/oci.rs:111-130` correctly demonstrate that GC refuses to
collect when reachability cannot be re-established.

The fallback is conservative (a raw JSON blob that resembles OCI can be
rejected), which is acceptable for integrity. It does not close the separate
bounded-work issue: `verify_live_graphs` still rehashes every protected graph
before each GC, while `max_entries` only bounds candidate/deletion rows. That
can turn a one-entry GC request into an unbounded scan and remains inconsistent
with the required bounded incremental GC contract.

## Qualification evidence reviewed

Prior artifacts show successful x86_64 and aarch64 builds/tests, including the
then-existing corrupt-reachability and SIGKILL-import tests. Prior churn
reports contain 100,000 blob operations and 20,000 OCI lifecycles on both
architectures. Throughput and idle reports include RSS, threads, FDs, and
latency/throughput data. `cargo audit` and `cargo deny` artifacts report no
known vulnerabilities and successful policy checks. These logs predate the
latest primary-root fallback and foreign-sidecar regression changes; final
latest runs are ongoing, so this round makes no native pass claim for those
new tests.

The final native reruns are now complete. The latest production source and
Cargo inputs independently compare equal between the local directory and the
`docs/native-{x86_64,arm64}-snapshot.json` records. The latest x86_64 and
aarch64 build/test logs pass the reviewed subset, including the foreign
sidecar and corrupt lost-root regressions; the suites report 18 test entries
including helper no-op targets. Latest core 100,000/20,000 churn,
throughput/idle, and state logs are also present. This closes the earlier
evidence-timing concern for that subset, but it does not qualify the absent
registry path, authenticated transfers, daemon API/journal churn, or the full
fault campaign (disk-full, power-loss, crash-after-publication,
crash-during-preparation/GC). Fuzz evidence remains short macOS smoke runs and
does not cover registry responses. The status files themselves still report
`production_qualified:false`.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL` — **do not approve release**. The
latest native subset now passes, while the round-three regression tests and
final evidence have been reviewed. Release remains blocked by absent registry pull/push and secure
credentials, missing BuildKit/Node cutover and Zig removal, bounded replay
semantics, the SQLite reopen TOCTOU, unbounded live-graph verification, and
missing registry/disk-full/crash fault qualification.
