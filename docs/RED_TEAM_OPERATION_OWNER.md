# Red-team review: durable operation ownership

Date: 2026-10-05  
Scope: durable `PeerIdentity` ownership in API operation journals, replay,
startup recovery, registry companion records, and retirement. This is a scoped
implementation review, not a release gate.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`. I found no confirmed mandatory owner,
replay, or registry-companion defect in the current implementation. The daemon
still authorizes only peers with its own effective UID; this change binds
allocated operation tokens to the exact `(uid, gid)` observed for that client,
but does not establish cross-UID admission or a service identity policy.

## Reviewed invariants

- The accept loop obtains `SO_PEERCRED` before packet handling, rejects a
  different UID, and passes the resulting UID/GID into dispatch. Allocation
  persists an owner; `begin` and `complete` compare the caller identity inside
  the same state transaction before changing intent, applying an effect, or
  recording a result.
- Replay checks owner before returning a terminal result. A foreign caller
  cannot use a valid operation token to trigger a retry, read its result, or
  change its terminal record through this API path.
- Startup recovery deliberately retains the persisted historical owner while
  converting interrupted intents to the explicit uncertain failure. This is
  internal reconciliation of durable state, not a client impersonation path.
- Registry operations write the operation and companion record in the same
  transaction. Startup audit requires exact companion equality, including the
  owner, phase, request, result, and sequence; non-registry operations must
  have no companion. Retirement removes both records together.
- Ownerless records remain representable for legacy decoding through
  `Option<PeerIdentity>`, but startup audit rejects them with an explicit
  migration error before binding the socket. The record is preserved, and the
  journal is not silently adopted or rewritten. Corrupt owner/sequence/range
  data also fails closed through the bounded audit.

## Evidence boundary and remaining work

Source review found no mandatory scoped finding. Public owner tests cover
foreign-owner mutation and replay refusal plus ownerless startup refusal and
preservation. The source invariant checks registry-companion equality,
including owner, but there is not yet a dedicated public test that changes
only the companion owner and proves startup rejection; that coverage remains
open. The UID-tamper/import fixture was corrected to open a separate read-only
FD and derive the requested size from that same FD. The two earlier attempts
remain invalid evidence because they failed on an unrelated input-size/EOF
mismatch. The current ARM64 and x86_64 native runs pass the corrected
foreign-owner import/replay test, ownerless startup refusal, the actual kernel
same-UID/different-GID operation-owner replay case, ordinary workspace tests,
clippy with `-D warnings`, and release all-target builds; their 103-file
snapshots match the reviewed trees (`docs/operation-owner-native-arm64.txt`,
`docs/operation-owner-native-x86_64.txt`,
`docs/operation-owner-final-arm64-snapshot.json`,
`docs/operation-owner-final-x86_64-snapshot.json`). The valid ARM64 pre-fix controls
reproduce both the missing-GID binding and ownerless-startup failures
(`docs/operation-owner-gid-pre-fix-arm64.txt`,
`docs/operation-owner-legacy-pre-fix-arm64.txt`). The UID-tamper/import
pre-fix control reaches CAS publication and returns an artifact fact instead of
rejecting the foreign owner (`docs/operation-owner-uid-pre-fix-arm64.txt`),
while the current test rejects it before publication.
The ownerless migration policy is intentionally fail-closed: deployment
requires an explicit migration procedure rather than guessing an identity for
old records.

These results qualify the operation-owner change on both native hosts, but do
not close the separate registry-companion owner-mismatch test or the broader
artifactd production gates.

Same-UID trust still allows any cooperating process with that UID to present
the same `(uid, gid)` identity. Cross-UID admission remains outside this
change and must not be inferred from the durable owner field. Full release
remains blocked by the project's independent ownership, fault/recovery,
caller-cutover, and other production qualification gates.
