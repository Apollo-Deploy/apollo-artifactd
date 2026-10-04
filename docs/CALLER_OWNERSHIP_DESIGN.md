# Caller ownership design

This is an implementation map for admitting trusted service peers with different UIDs. It preserves the existing v2 operation token format and replay semantics while making every durable capability owner-bound. It is a design document only; no production files are changed by this investigation.

## Current boundary and threat

`src/api/mod.rs::serve` currently accepts a connection only when `rustix::net::sockopt::socket_peercred(&socket)?.uid == geteuid()`. The returned Linux `UCred` contains `pid`, `uid`, and `gid`. Once producer and sandbox consumer UIDs are admitted, UID equality can no longer be the authorization policy. The socket must map `(uid,gid)` to a configured role and reject unknown peers before decoding or journaling a request.

The operation journal in `src/api/journal.rs` binds an ID to the current epoch, sequence, and serialized action. It does not bind the ID to the peer. A token copied from a producer to another admitted peer can therefore be replayed, and `OperationAllocate` can be used by any admitted peer to consume the global 4096-record window. Pins and leases in `src/state/records.rs::Reference` contain only a digest; `Store::unpin`, `Store::release`, `Store::leased`, and `Store::open_prepared` currently authorize by opaque ID alone. A peer that learns another peer's lease can open its graph or release it; a peer that learns a pin ID can revoke GC protection.

## Durable records and schema

Add a generic persisted peer identity and keep authorization policy separate:

```text
PeerIdentity = { uid: u32, gid: u32 }
EffectiveRole = Producer | ReadConsumer | Admin
```

Persist UID and GID as the durable owner. PID is connection evidence and diagnostic metadata only; it must not be the durable owner because retries and recovery cross process restarts. Resolve the effective role from current configuration for each connection. Do not persist role as part of owner equality: an operator role change must not transfer ownership or break a legitimate same-UID retry, although it may grant or remove permission for the next action.

Extend `Operation`, `Reference`, and any new lease-grantee record with `owner: Option<PeerIdentity>` during an additive schema revision. Existing records without an owner are **legacy-unowned**. They must fail closed for cross-UID mutations, replay, lease use, and release. Do not assign them to the first caller. Provide an explicit admin-only migration/adoption operation if legacy stores must be retained. A schema bump is preferable to silently changing the meaning of v2 records; the v2 `OperationId` text format and epoch/sequence validation remain unchanged.

For delegated leases, persist both `owner: PeerIdentity` (creator) and `grantee: PeerIdentity` (the only peer allowed to open). If the creator is also the consumer, set them equal. A plain owner release is safe only before the consumer claims the lease. This is mandatory for prepared directory FDs: recursively deleting a prepared tree can remove child paths that a consumer still needs through an open directory FD.

Use an explicit persisted lease state, for example `available -> claimed -> released`. `OPEN_BLOB`/`OPEN_PREPARED` must transactionally move the lease to `claimed` before publishing the FD. Before `claimed`, the owner may cancel the lease. After `claimed`, only the grantee may complete/release it, and GC must retain the complete prepared tree until that completion is durable. Admin forced revoke is exceptional: it may mark cancellation but must retain the claimed lease's GC protection until the grantee acknowledges completion; silently deleting a claimed prepared tree is forbidden. If the grantee disappears, recovery must preserve the claim rather than guess that all FDs closed. Never encode Apollo build or workload IDs in these records.

## API callsite changes

Capture `PeerIdentity` and its current effective role immediately after `accept_with` and pass them through the request boundary. Keep `Request` and `OperationId` wire shapes unchanged; the server derives the peer identity from `SO_PEERCRED`.

Required signature changes:

```text
api::dispatch::execute(store, request, fd, caller)
api::dispatch::recorded(store, request, fd, caller)
api::journal::allocate(store, caller)
api::journal::begin(store, operation_id, payload, registry, caller)
api::journal::complete(store, operation_id, payload, outcome, registry, caller)
Store::pin(caller, id, digest)
Store::unpin(caller, id)
Store::lease(owner, id, digest, grantee)
Store::release(caller, id)
Store::leased(grantee, id, digest)
Store::open_prepared(grantee, id, lease)
```

`admit_oci_pinned`, `import_oci_archive_pinned`, and registry `pull_pinned` must receive the caller identity and effective role, then create/check the pin in the same redb transaction as graph admission. Internal GC and startup recovery use an explicit internal principal and must not impersonate a socket peer. Existing direct `Store` tests can use a constructed identity only at the public Store boundary; protocol integration must use real peer credentials.

## Role policy

Apply policy before journal allocation/begin, and repeat ownership checks inside the transaction:

| Role | Allowed examples | Forbidden examples |
| --- | --- | --- |
| producer | blob/OCI import, prepare, pin, pull/push, ensure-local | foreign lease use/release, GC/admin recovery |
| read-consumer | status, inspect, verify, resolve, delegated lease create/use/release, open FDs | imports, pins, unpin, pull/push, GC, reconcile, doctor |
| admin | maintenance, doctor, reconcile, GC, explicit foreign-reference revoke/adoption | customer execution or arbitrary registry credentials unless separately configured |

`OPEN_BLOB` and `OPEN_PREPARED` are observations at the journal layer but remain capability-bearing operations: enforce the lease owner/grantee before opening any FD. `Pull` and `Push` remain registry-journaled mutations and require a producer/admin role; registry credentials never enter the durable request payload.

## Journal rules and boundedness

`allocate` stores the caller identity with the allocated record. `begin`, replay, and `complete` load the record and require exact UID/GID equality before comparing or writing the action payload. A different UID/GID must receive an authorization error without changing the record or journal counters. Same UID/GID replay after process restart remains valid, preserving current retry behavior. `begin` must evaluate the decoded action against the current effective role before persisting intent; allocation alone grants no action permission.

Recovery scans do not have a peer. They validate the persisted owner as data and use the internal recovery principal only for the daemon's own terminal-uncertain transition. An ownerless legacy intent is not reassigned; recovery should mark it uncertain or quarantine according to the existing migration policy.

The 4096 global journal window remains bounded. `read-consumer` must be able to allocate tokens for journaled lease create/release, so allocation cannot be restricted to producer/admin. Enforce a bounded per-identity outstanding allocation quota and retain the global window; a peer that exceeds its quota is rejected before redb allocation. At `begin`, enforce the action role policy, so a read-consumer token cannot be turned into an import, pin, GC, or registry mutation. Allocation alone still consumes bounded journal capacity, so the quota and global limit are part of the denial-of-service boundary.

All ownership checks and reference mutations must share one redb transaction. For `LEASE_CREATE`, persist owner/grantee and digest atomically. For release/revoke, read the record and apply the lifecycle transition in the same transaction; do not delete a claimed lease on owner release. For open, authorize the lease, mark it claimed, and commit before the existing graph traversal and filesystem open; a stale or foreign lease must not reach FD publication. If the later open fails before an FD is sent, clear the claim in a follow-up transaction. If the process dies between claim and open, leave the claim protected for recovery rather than guessing that no consumer can still use it.

## Compatibility and migration

The current schema records and v2 operation tokens are not sufficient to infer ownership. Existing ownerless pins, leases, and operations must therefore be treated as untrusted legacy state after the schema revision. Startup may continue read-only inspection, but mutating or capability-bearing access must fail closed until an explicit admin migration binds each record to a configured principal. Never infer ownership from the current socket UID, filesystem owner, opaque ID prefix, or operation token epoch.

## Qualification at the public boundary

The strongest regression is a Linux integration test with a real artifactd process under its dedicated UID and separate producer, read-consumer, and admin client processes. It must use the actual Unix `SOCK_SEQPACKET` socket and verify:

1. A producer allocation/token cannot be begun or replayed by the read-consumer; producer replay still returns the recorded result after restart. A role change for the same UID does not change durable ownership, while the current policy still gates new actions.
2. The read-consumer can allocate and complete its own journaled lease create/release operations, but cannot begin an import, pin, unpin, pull, push, GC, reconcile, or doctor action; per-identity quota and the global window remain bounded.
3. A producer-owned lease cannot be opened or released by the consumer before it is delegated; a delegated lease can be opened only by its grantee, remains GC-protected while claimed, and is released only by consumer completion.
4. A consumer cannot revoke producer pin protection, while an explicit admin revoke succeeds.
5. Cross-UID racing release/open attempts leave one authorized result and no unauthorized deletion or FD.
6. Unauthorized allocation attempts do not advance `next_sequence` or consume the 4096-entry window.
7. Owner cancellation before claim is collectible, while owner cancellation after claim is rejected and the prepared tree remains protected until grantee completion; admin forced revoke has the same protection rule.

Retain `tests/api.rs` and `tests/journal_api.rs` as same-peer v2 idempotency/conflict coverage. Multi-UID tests require native Linux credentials. A root-run qualification harness can launch fixed numeric UIDs in isolated temporary store/runtime directories with `setresuid`/`setresgid`; it need not create Unix accounts. The daemon must still run unprivileged under its configured artifactd UID, and the test must use the real `SO_PEERCRED` boundary rather than an injectable identity seam.
