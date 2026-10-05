# Config-lease red-team review — 2026-10-05

## Scope

This is a scoped adversarial review of the durable BuildKit configuration-blob
lease path staged into the isolated x86 publisher/transports checkout. It
covers the publisher lease intent and recovery files, the config-blob FD
verification, and the BuildKit publication, solve, and restart-recovery paths.
The review was performed after reading the `plan-implementation-red-team` and
`test-audit` skill instructions. No test-only production seam was introduced
and no new test was added: the review findings are about the existing public
publisher/transport boundary and the existing native cancellation and restart
qualification evidence.

## Exact source revisions

The staged files were copied into
`/home/tihan/artifactd-legacy-integration-20261005/APIs/apollo-deployment-api`
on `tihan-apollo`. The relevant file SHA-256 values were:

| file | SHA-256 |
| --- | --- |
| `crates/apollo-buildkit-publisher/src/config_lease.rs` | `9217e65379596b3299308e4b07ed68768149bfc4fb73bfc990496e66db59bc0c` |
| `crates/apollo-buildkit-publisher/src/config_blob.rs` | `1e8d0c40b43270f640f4cfc2fe795dce77b7acadaac140c2dd9c6708cdedfa65` |
| `crates/apollo-buildkit-publisher/src/intent_files.rs` | `aa97c7a9ba2e3c6cfe7ab7f4d55f369bdbe6c9fbb218c8360273b61d8fb4cd55` |
| `crates/apollo-transports/src/buildkit/publication.rs` | `41926522a67f8f1242a0499e3900424b913017e80ff3e93719b264470e04b23a` |
| `crates/apollo-transports/src/buildkit/solve.rs` | `cc27f07ca63574e08025f23a3a065302342b5a51a10f265a709cca1553da716e` |
| `crates/apollo-transports/src/buildkit/recovery.rs` | `b61594b553c977666a1b4b0669e5b206737fd7f9e8597b8963a385bd1119f402` |

The publisher module export was also staged so `config_lease` is compiled:
`crates/apollo-buildkit-publisher/src/lib.rs` (`f63ad7088057c9ec9212e1c94ea3371116e2c681f780a34b3586c953b98e2ec5`,
`mod config_lease` and its public reconciliation export).

## Build evidence

On `tihan-apollo`, with a private target directory and no production service:

```text
CARGO_TARGET_DIR=/home/tihan/artifactd-legacy-integration-20261005/publisher-transports-target \
CARGO_INCREMENTAL=0 cargo check -j2 -p apollo-buildkit-publisher -p apollo-transports --locked
Finished dev profile [unoptimized + debuginfo] target(s)
exit 0
```

The check compiled the publisher and transport crates together after the module
export was staged. The canonical V048 migration SQL was restored before this
review; this work did not change migration bytes or applied migration history.

## Findings

### M1 — cleanup can hold an attempt open indefinitely

`run_attempt` and restart recovery retry `reconcile_config_lease` until it
succeeds. The retry sleeps for five seconds, so an unavailable artifactd or a
permanently corrupt private intent prevents the BuildKit attempt from reaching
a terminal journal state and retaining cleanup state indefinitely. This is a
deliberate fail-closed ownership policy and avoids publishing while a lease may
still be live, but it is an availability and operator-recovery risk. A bounded
retry budget with an explicit `cleanup-uncertain` terminal state and a durable
operator reconciliation action is needed before claiming convergence.

### M1 — release uses the unbounded/default client control path

`ConfigLease::begin` uses `allocate_with_control`, but `release` uses
`Client::allocate` and `Client::call` without the caller's wall-time and
cancellation callback. During cancellation or shutdown, cleanup can therefore
wait for the client's default response timeout rather than the attempt's
deadline. The persisted release intent makes retry safe, but the missing control
budget is still a liveness gap. The release path should accept the same control
callback/deadline contract and preserve the ambiguous operation on cancellation.

### M2 — receipt provenance is persisted but not checked during reconciliation

The intent stores `receipt_digest`, but `validate` checks only schema version,
socket, server UID, and the operation-derived lease ID. The private 0700 intent
directory and lock substantially constrain substitution, and releasing a stale
lease before starting a replacement is safe under that ownership boundary.
Still, a path reused with a different receipt cannot be distinguished by the
reconciler. Bind reconciliation to the expected receipt/manifest where the
caller has that fact, or document and enforce that the directory is one-shot.

### M2 — restart cleanup is safe but can issue repeated release operations

If a release effect succeeded but its response was lost, the intent remains
`released=false`; a later restart allocates a new release operation and retries
the same lease ID. The server-side release is idempotent and the operation
token is persisted before the effect, so this is safe. It does increase journal
pressure during repeated outages and should be covered by the existing bounded
operation-window policy and metrics.

### Accepted controls

The review found these controls correctly placed:

* Lease IDs are derived from the persisted create `OperationId`; callers cannot
  choose or retarget an ID.
* The create intent is written before the lease-create effect, and release
  intent is written before the release effect.
* Config blobs are bounded to 4 MiB, opened through a verified FD, checked for
  regular-file ownership/links, exact size, and SHA-256 before publication.
* Intent and lock files use `openat`, `NOFOLLOW`, `CLOEXEC`, private modes,
  ownership checks, non-blocking locking, temporary `EXCL` publication, and
  directory fsync.
* Recovery performs sandbox and config-lease reconciliation before exposing a
  recovered terminal result.

## Test-audit disposition

No test was added in this review. A new test would have duplicated existing
public cancellation/SIGKILL/restart/recovery/GC qualification rather than
covering a newly identified externally observable regression. The M1 findings
need an explicit product decision and a public bounded-cleanup contract before
writing a credible regression test.

## Gate status

## Follow-up implementation

The scoped liveness fixes were then applied in the publisher lease owner. The
updated `config_lease.rs` SHA-256 is
`c261f890fe41bd518443a43546b16a3f73250d841266cd433db64db7dc980c2c`.
Release allocation and release requests now use a private five-second control
deadline, independent of user cancellation. The intent is written before the
release effect and remains present with `released=false` when the deadline is
exceeded. Reused intent paths are checked against the current receipt digest
and manifest during `begin`; reconciliation without receipt facts remains
limited to the persisted socket/server/operation identity.

The same isolated x86 checkout was rechecked after this change:

```text
CARGO_TARGET_DIR=/home/tihan/artifactd-legacy-integration-20261005/publisher-transports-target \
CARGO_INCREMENTAL=0 cargo check -j2 -p apollo-buildkit-publisher -p apollo-transports --locked
Finished dev profile [unoptimized + debuginfo] target(s)
exit 0
```

The follow-up still does not constitute independent red-team approval. Native
publisher cancellation/restart evidence remains parent-owned qualification, and
the full BuildKit fixture has not yet been rerun against a freshly rebuilt
control-plane binary from the restored canonical migration source.

The transport foreground cleanup loop was subsequently bounded to 30 seconds.
On timeout it journals `cleanup_uncertain`, retains the attempt directory and
lease intent, and returns the worker instead of sleeping forever. Restart
recovery remains the durable reconciliation owner. The same x86 package check
passed after that change. A strict Clippy run reached the changed crates but
was blocked by two pre-existing `apollo-domain` `too_many_arguments` errors in
certificate issuance/revocation; those files are outside this scope.

The first implementation used one maintenance thread per failed attempt. That
was replaced with one process-wide bounded `SyncSender` queue and cleanup
worker, sized to the configured active-attempt limit. Failed foreground
cleanup keeps the attempt nonterminal and queues its original result; the
worker retries with bounded reconciliation waits and settles only after the
lease and sandbox cleanup succeed. The transport files in that follow-up are:

```text
buildkit/mod.rs      735ceadaeaecfcd73978a3fd8a7b2c3e36f6ad57ae10422926f966f6e9752fc3
buildkit/executor.rs 38f81731676bec125d51cb1f23c552b2226e26a5dad1b4032d8886057ee769f6
buildkit/solve.rs    06a30b0a56369d9fdd8e05c2c51230b31a517d3d531e1311a09b361ccbed37dd
```

The focused package check passed on `apollo-node-01` using its configured
Cargo path and private target directory. The x86 host was intentionally not
rebuilt while its free space was below 200 MiB.

`RED_TEAM_CONFIG_LEASE_REVIEW_NON_CONVERGED`

This is one scoped review round, not independent red-team approval. The compile
passed, but the two M1 liveness findings remain open and no global production
approval is claimed.

The bounded-owner implementation was then tightened for restart recovery. The
cleanup worker now keeps a bounded round-robin pending queue, performs one
finite reconciliation pass per job, and requeues incomplete work. Recovered
attempts are submitted to that same owner after the worker starts; startup no
longer performs an unbounded per-attempt cleanup loop before the maintenance
owner exists. Current transport source hashes are:

```text
buildkit/mod.rs      735ceadaeaecfcd73978a3fd8a7b2c3e36f6ad57ae10422926f966f6e9752fc3
buildkit/executor.rs 587005af1301215e80771662be560948f4ba2052107b70a57b4170f610bb192f
buildkit/recovery.rs 9fbf75f0934ee9aa37531e2d176a611b1af0c1452848c3b42f6d06b4fd0a7e3a
buildkit/solve.rs    b61f4cb0589c8db1b68b25a8067c0c0b251f60d3dd75121a8043e974ae2cc83a
```

`cargo check -j2 -p apollo-buildkit-publisher -p apollo-transports --locked`
passed on `apollo-node-01` using the private target directory. No two-job
outage qualification was run in this round; the gate remains non-converged.
