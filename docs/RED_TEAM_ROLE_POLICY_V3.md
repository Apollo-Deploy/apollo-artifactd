# Red-team review: V3 peer roles, policy, and lease claims

This is an independent, read-only review of the current uncommitted V3 protocol,
peer policy, Unix admission, credential descriptor, and delegated lease changes.
It is a scoped report and is **not** a release approval. The review was performed
against the current source after the lease implementation landed; `cargo check`
passes locally. No native or privileged qualification is claimed here.

## RP3-001 disposition — prior ABA concern withdrawn

The initial draft of this report treated lease-row deletion without tombstones as a
mandatory ABA finding. That conclusion did not account for the current V3 wire
contract and the subsequent lease-identity decision. `LEASE_CREATE` carries no
caller-supplied lease ID; the daemon derives `lease_<operation_id>`. The
persisted journal allocates strictly increasing sequences, rejects retired tokens,
and retains the epoch, while local direct lease creation uses its own persisted
monotonic namespace. A delayed release therefore cannot target a later lease
under the current reachable daemon API. The prior HIGH finding is withdrawn.

The previously noted defense-in-depth gap is resolved in the current source.
`Store::lease_for` now accepts an `OperationId`, derives the `LeaseId` internally,
and in one transaction requires a live owned operation intent whose serialized
`Action::LeaseCreate` matches the digest and grantee (`src/cas/leases.rs:59-117`).
The dispatch path passes the operation token directly. No current public V3
creation path accepts an arbitrary lease identity. A public delayed-release and
exact-ID recreation regression remains useful validation, but no current ABA
finding remains.

## Controls that held in source review

* The daemon obtains Linux `SO_PEERCRED` before policy admission
  (`src/api/mod.rs:61-70`), and the policy maps the exact `(uid,gid)` pair. A
  policy file is an absolute, bounded, no-follow, daemon-owned private regular
  file (`src/api/policy.rs:35-88`). The runtime directory and stale socket are
  checked for daemon ownership, configured group, and expected mode before
  removal/bind (`src/api/runtime.rs`, `src/api/mod.rs:29-58`).
* Request role authorization occurs before journaling or dispatch
  (`src/api/mod.rs:87-93`). Wire-supplied lease grantees are resolved against
  the configured peer map; a consumer cannot delegate to another peer
  (`src/api/policy.rs:107-170`). The stored grantee is checked against the
  kernel caller, so a wire identity does not substitute for peer credentials.
* Operation owner checks remain UID/GID exact in the durable journal. The lease
  creation ID is derived from the allocated operation token, and both local
  lease sequence and journal sequence are monotonic. I found no ordinary
  allocator-level ID collision in this pass.
* Credential descriptors are bounded private regular files and are checked for
  the caller's UID before JSON parsing (`src/registry/credentials.rs:38-55`).
  Secrets are held in zeroizing buffers and are not serialized into the
  operation payload. This is an owner-UID check, not proof of who originally
  opened an FD; same-UID peers are already one Unix file authority. A dedicated
  cross-role same-UID credential test remains desirable if such a policy is
  supported.
* Lease claim is committed before opening the blob or prepared directory
  (`src/cas/leases.rs:143-167`, `src/prepare/mod.rs:132-156`). An owner may
  release only an available lease; a claimed lease may be released only by its
  grantee (`src/cas/leases.rs:108-117`). GC scans lease rows as roots, so a
  claimed row remains live. Failures after the claim conservatively retain the
  claim rather than guessing that no FD escaped.

## RP3-002 disposition — descriptor-anchored bind resolved

The prior absolute-path bind finding is resolved in the current source.
`src/api/runtime.rs::bind_socket` now constructs a `/proc/self/fd/<parent-fd>/<name>`
address from the captured runtime directory and binds through that capability.
`finalize_socket`, stale-entry removal, locking, and directory sync remain relative
to the same `Dir`, so replacing a writable ancestor cannot redirect the bind or
post-bind ownership changes. The focused captured-runtime rename regression is
reported passing on both native x86_64 and arm64 hosts.

The remaining operational constraint is pathname length: the proc-fd anchor adds
bytes to the Unix socket address and may reject unusually long configured socket
paths that would have fit the original pathname. This is availability/configuration
hardening, not a demonstrated ownership bypass; a short-path validation/error would
make the failure explicit.


## Remaining qualification gaps

No claim is made here for delegated lease public-boundary tests, persisted owner
corruption, restart/SIGKILL between claim and FD publication, cross-UID policy
runs, registry credential fault cases, caller cutover, full fault/fuzz/churn/
performance evidence, or removal of Zig and sibling Apollo dependencies. The
repository's existing qualification documents continue to define those broader
release gates. Verdict: `APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`; do not emit
`RED_TEAM_RELEASE_APPROVED` from this scoped review.

