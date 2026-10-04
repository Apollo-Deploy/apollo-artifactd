# Red-team review: reference ownership and caller propagation

Date: 2026-10-05  
Scope: reference records, pin/lease authorization, OCI admission/archive,
registry pull, preparation, GC marking/sweep, and API dispatch. This is a
scoped implementation review, not a release gate.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`. I found no confirmed foreign-owner
reference adoption, same-digest pin takeover, legacy-reference destructive
collection, or caller-propagation bypass in the reviewed paths.

## Reviewed invariants

- New pins and leases persist the supplied caller identity. Existing records
  require both owner equality and digest equality inside the transaction;
  same-digest idempotency cannot transfer ownership. Unpin, release, and lease
  reachability authorize the same owner before mutation or access.
- OCI admission validates a requested pin before graph writes, checks an
  existing pin owner in the same transaction, and writes the pin with the
  caller identity. Archive import and registry pull pass the API caller into
  the common admission path. Public Store wrappers use the process identity
  deliberately for trusted in-process callers.
- GC root marking requires owners on every pin and lease before verifying or
  collecting content. Legacy ownerless references therefore fail closed rather
  than being adopted or deleted. Prepared records are handled separately and
  remain protected by the existing prepared mark/recovery rules.
- Reference records are optional-owner for decoding legacy state, but
  `authorize` and `require_owner` reject missing ownership. No reviewed path
  silently fills an owner on first use.
- Descriptor graph writes are additive and digest-pinned; a foreign pin cannot
  be replaced by a matching or conflicting digest. Failed authorization occurs
  before reference mutation, while pre-transaction blob/graph verification is
  read-only.

## Evidence boundary and remaining work

Source review found no confirmed destructive ownership bypass. The API caller
is propagated through pin, lease, OCI admission, archive import, registry
pull, prepare, open-blob, and open-prepared paths. `Store::prepare` retains a
process-identity public wrapper for trusted in-process callers, while API
dispatch uses the caller-aware `prepare_for` entry point. Cross-UID role policy
and consumer lifetime/delegation are not implemented.

Focused native qualification now passes on ARM64 and x86_64 with matching
104-file source snapshots. The public tests cover 65 preceding owned
references, explicit ownerless-reference GC refusal without blob deletion, and
real kernel same-UID/different-GID attempts to adopt same-digest pins and
leases. They also verify unpin/release and `OPEN_BLOB` refusal before and after
SIGKILL, fresh-owner token allocation, rightful retry, GC protection, and
eventual collection (`docs/reference-owner-focused-arm64.txt`,
`docs/reference-owner-focused-x86_64.txt`). Controlled ARM64 pre-fix runs
showed the UID-only comparator accepting an unauthorized request and the
removed legacy guard failing to reject ownerless GC state; both production
guards are restored (`docs/reference-owner-gid-pre-fix-arm64.txt`,
`docs/reference-owner-legacy-pre-fix-arm64.txt`).

Coverage remains open for direct persisted-owner tampering, foreign-owner
`OPEN_PREPARED` with a covering lease, and a registry-specific caller-owner
test. Source propagation through those paths is verified, but these cases
have no dedicated focused proof yet. Full release remains blocked by the
project's ownership, fault/recovery, caller-cutover, and other production
qualification gates.

Both final native ordinary workspace/all-target suites, warnings-denied
Clippy, and release/all-target builds pass. Final ARM64 and x86_64 snapshots
match all 104 current source inputs (`reference-owner-final-*`). This does
not broaden the focused coverage above to ignored fault/registry/churn
campaigns or grant full release approval.
