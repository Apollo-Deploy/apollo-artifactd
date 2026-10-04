# Delegated lease claim state machine

This is a read-only implementation investigation for delegated blob/prepared-root leases. It describes the smallest safe change from the current source; it does not edit production files.

## Current protection path

`src/cas/references.rs` currently stores `Reference { digest, owner }`. `leased_for` authorizes the caller as the owner and then traverses the OCI graph. `src/prepare/mod.rs::open_prepared_for` checks that lease before opening and verifying the prepared directory FD. `src/gc/mod.rs` marks every row in the `leases` table as a handle root, and `src/prepare/recovery.rs`/`src/gc/prepared.rs` delete a prepared tree only after a ready snapshot proves no active handle protection.

There is no grantee, claim state, or lease generation today. A producer cannot safely delegate a lease to another UID, and an owner release after `OPEN_PREPARED` could remove the only GC root while the consumer still relies on the directory FD. A released ID can also be recreated and allow a delayed old `LEASE_RELEASE` to target a new lease (ABA).

## Wire contract

Add a generic protocol identity and an optional recipient to `Action::LeaseCreate`:

```text
PeerIdentity { uid: u32, gid: u32 }
LeaseCreate { id, digest, grantee: Option<PeerIdentity> }
```

The optional field is serde-defaulted for existing clients. `None` means the caller is both owner and grantee. A supplied identity is accepted only when the caller's configured role is allowed to delegate to that exact recipient. The daemon converts it to the state identity; it never treats a wire UID/GID as proof of authorization. The operation payload includes the optional field, so v2 journal identity conflict detection covers recipient changes.

`LeaseRelease` remains a logical completion acknowledgement. `OPEN_BLOB` and `OPEN_PREPARED` identify the lease as they do today. The smallest protocol-compatible ABA defense is lease-ID retirement: once an ID is released or canceled, retain a terminal tombstone and reject any later create using that ID. The existing 100,000 reference bound applies to live rows plus tombstones; when exhausted, fail closed. Do not delete tombstones merely because no current FD is visible. A future lease capability nonce can compact this state, but it requires adding a nonce/token to every open and release request and is a larger protocol change.

## Persisted records

Extend `Reference` with serde-defaulted fields:

```text
owner: PeerIdentity
grantee: PeerIdentity
state: Available | Claimed | Released
```

Legacy ownerless rows remain unusable until explicit migration. Pins continue to use owner and a non-lease state; leases require owner, grantee, and state. A released lease tombstone retains the original digest, owner, grantee, and ID, with no GC root because it is no longer active. It must still be validated as a tombstone so it cannot be retargeted.

## State transitions

```text
create(None)       -> Available(owner=caller, grantee=caller)
create(grantee)    -> Available(owner=caller, grantee=configured-recipient)
open by grantee    -> Claimed
owner cancel       -> Released tombstone (Available only)
grantee completion -> Released tombstone (Claimed only)
admin revoke       -> cancellation-requested/Claimed until grantee completion
```

The owner may cancel only an `Available` lease. Once claimed, owner release is rejected because it could make a prepared tree collectible while the grantee still has an open directory FD. The grantee is the only normal actor allowed to complete a claimed lease after it has closed all returned FDs. Admin forced revoke must preserve the claimed lease's GC protection until that completion; it cannot turn a claimed row directly into a collectible tombstone.

`OPEN_BLOB` and `OPEN_PREPARED` must transactionally authorize the grantee and change `Available` to `Claimed` before opening or publishing the FD. If opening fails before any FD can be sent, a follow-up transaction may return the row to `Available`. If the daemon dies after the claim commit, or if `SCM_RIGHTS` response delivery fails with unknown delivery status, leave it `Claimed`; recovery cannot prove that no FD escaped. A claimed lease may be reopened by its same grantee, and completion remains the consumer's explicit acknowledgement after all such FDs are closed.

The lease row remains a GC root in both `Available` and `Claimed` states. Only durable consumer completion creates the `Released` tombstone. The next GC epoch then observes no lease root and can remove an unpinned prepared tree. Existing prepared recovery already fails closed when a GC snapshot reports active handles; the new lease state must preserve that behavior rather than filter claimed rows out of marking.

## Smallest API changes

Keep trusted local wrappers for direct Store callers, deriving the current process identity. Add caller-aware methods used by dispatch:

```text
Store::lease_for(id, digest, owner, grantee)
Store::claim_lease_for(id, digest, grantee)
Store::release_for(id, caller)             // Available owner or Claimed grantee
Store::leased_for(id, digest, grantee)     // authorization only
Store::open_prepared_for(id, lease, grantee)
```

`claim_lease_for` should be called from `open_prepared_for`/`OPEN_BLOB` in the same serialized Store ownership model. `src/api/dispatch.rs` supplies the peer identity and configured recipient policy. `src/prepare/mod.rs` must claim before directory open. `src/gc/mod.rs` must scan and validate both lease states. `src/prepare/recovery.rs` must preserve claimed leases across restart and must never infer completion from process death.

Each create/release/claim transition is journaled by the existing operation token. Operation owner remains UID/GID-bound; lease grantee is a separate capability recipient. Same ID with a different digest, owner, grantee, or tombstone state is an explicit conflict, never an idempotent success.

## Qualification cases

At the actual Unix API boundary, qualify with distinct numeric UIDs and real `SO_PEERCRED`:

1. Caller-created lease defaults owner and grantee to the caller.
2. Configured producer delegation permits only the configured consumer UID/GID; an arbitrary wire recipient is rejected before journaling.
3. Consumer open changes `Available` to `Claimed`; owner release then fails.
4. Consumer completion after closing its FD creates a tombstone; the next GC epoch may collect the unpinned prepared tree.
5. SIGKILL after claim and before FD response leaves the lease claimed and the prepared tree protected after restart.
6. A delayed release for a retired lease ID cannot release a newly created lease because the ID cannot be reused.
7. GC, prepared recovery, and admin revoke never delete a claimed prepared tree without consumer completion.

