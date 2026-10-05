# Bounded lease identity and ABA decision

Protocol V3 implements daemon-derived lease identities:

```text
lease_id = "lease_" + create_operation_id
LEASE_CREATE { digest, grantee? } -> { lease_id }
```

There is no caller-selected-ID creation path or protocol-V2 fallback. The
operation journal persists an epoch, monotonically increasing sequence and
retirement floor. Retained create retries return their original receipt;
retired tokens reject effects. A new create always receives a different ID.
An old release therefore cannot remove a later lease, without an ever-growing
table of tombstones. The journal metadata key `operation_journal_v2` names its
existing storage encoding; it does not enable protocol-V2 requests.

`Store::lease_for` accepts an `OperationId`, derives the lease ID internally,
and in the same transaction requires a live owned journal intent whose action,
digest and effective grantee match the proposed reference. It cannot accept an
arbitrary lease string. The live lease limit remains 100,000 records.

Trusted in-process `Store::lease(digest)` separately mints
`local_lease_<epoch>_<sequence>` IDs. Its issuer metadata and reference publish
in one redb transaction. Existing issuer state must have both a canonical epoch
and a positive sequence; partial state or exhausted sequences reject creation.
This domain is separate from service operation-derived IDs. The previous public
`Store::lease(id, digest)` interface has been removed.

The optional wire `PeerIdentity { uid, gid }` is validated against configured
service roles before journaling. Kernel `SO_PEERCRED` determines the caller.
The serialized action remains the exact replay identity.

Leases transition from available to claimed before any blob or prepared FD is
published. A claim stays durable across failures, disconnect and restart.
Available leases permit creator cancellation; claimed leases permit only
grantee completion. Release removes the row, and its ID is never reissued.
No failed-open rollback guesses whether an earlier FD still exists.

`tests/delegated_lease_peer.rs` owns public daemon handoff and replay coverage.
`tests/local_lease.rs` separately owns the local issuer's restart and corrupt
metadata invariants. The ARM controlled guard-removal regression is retained in
`lease-v3-counter-control-arm64.txt`: removing the counter check recreated a
released identity and failed the public test; restoring it passed.

This identity decision does not qualify the complete caller cutover, fault,
fuzz, churn, performance or release gates. See `DELEGATED_LEASES.md` and the
qualification report for the exact native evidence.
