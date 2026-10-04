# Reference ownership implementation

This document maps the next narrow change: persist ownership for CAS pins and leases while preserving the already implemented operation-token owner binding. It does not widen the server admission policy by itself. The current `LEASE_CREATE { id, digest }` wire shape has no recipient field; explicit producer-to-consumer delegation requires the protocol addition described below.

## Baseline

`src/state/records.rs` already defines `PeerIdentity { uid, gid }`, and `Operation.owner` is persisted and checked by `src/api/journal.rs::allocate`, `begin`, and `complete`. `src/api/mod.rs` still admits only the artifactd UID, so reference ownership must be completed before trusted producer and consumer UIDs are admitted.

The remaining unowned surface is `Reference` in `src/state/records.rs`. `src/cas/references.rs` currently stores only `{ digest }`; `unpin`, `release`, and `leased` address records by opaque ID. Pin creation also occurs inside the graph transaction in `src/oci/admission.rs`, `src/oci/archive.rs`, and `src/registry/pull.rs`, so all of those paths must carry the same owner identity into the transaction.

## Record shape and compatibility

Extend the existing JSON record with optional fields so the redb table names and generic state API stay unchanged:

```text
Reference {
    digest: ArtifactDigest,
    owner: Option<PeerIdentity>,
    grantee: Option<PeerIdentity>,       // leases only
    lease_state: Option<Available|Claimed>, // leases only
}
```

Pins require `owner`, with `grantee` and `lease_state` absent. Leases require owner, grantee, and state. Use serde defaults only to decode old rows; an ownerless row is legacy state and must fail closed for unpin, release, lease use, and GC-sensitive lifecycle decisions. Never bind a legacy row to the first caller or to the current filesystem owner. An explicit admin migration is required to adopt it.

This is an additive schema revision from the current state schema. Existing operation owner checks remain unchanged. The revision must create or validate no new engine tables and must preserve the current bounded record-size checks.

The protocol should define its own generic optional recipient type, for example `artifactd_protocol::PeerIdentity { uid, gid }`, and add `#[serde(default, skip_serializing_if = "Option::is_none")] grantee: Option<PeerIdentity>` to `LEASE_CREATE`. The daemon converts that wire value into the state `PeerIdentity`; it never trusts the requested UID/GID by itself. `None` preserves existing clients and means the caller is both owner and grantee. A supplied recipient must match a configured allowed service recipient for the caller's role, or the request is rejected before journaling. The optional field is generic transport identity only and carries no Apollo business ID.

## Store API

Keep direct `Store` callers as trusted in-process local producer callers, without adding a test-only principal hook. The existing public convenience methods can derive `PeerIdentity::current()` from the actual process UID/GID and create new records owned by that identity:

```text
Store::pin(id, digest)                 // local trusted producer wrapper
Store::lease(id, digest)               // local owner == local grantee wrapper
Store::unpin(id)
Store::release(id)
Store::leased(id, digest)               // local grantee wrapper
```

Add caller-aware internal methods used by the Unix API and registry paths:

```text
Store::pin_for(owner, id, digest)
Store::unpin_for(owner, id, role)
Store::lease_for(owner, grantee, id, digest)
Store::release_for(caller, id, role)
Store::leased_for(grantee, id, digest)
Store::open_prepared_for(grantee, id, lease)
```

The wrappers are trusted local service APIs, not a way to adopt old records. If a wrapper encounters an ownerless existing row, it returns the same explicit migration error as the caller-aware path. The socket dispatcher must use the caller-aware methods exclusively once peer roles are enabled.

## Transaction rules

All reference operations read, authorize, and write in one redb transaction.

- Create: validate ID and graph, then write `{ digest, owner }` for a pin or `{ digest, owner, grantee, lease_state: Available }` for a lease. Existing same-ID/same-digest is idempotent only when the stored owner and, for leases, grantee match. A foreign owner is an authorization error even when the digest matches.
- Unpin/release: load the row, reject missing/legacy/foreign owner, then remove or transition it in the same transaction. Admin may explicitly revoke a foreign unclaimed reference. No caller may silently replace its owner.
- Lease use: compare the requested grantee to the stored grantee before graph traversal or FD publication. The claimed prepared-tree lifecycle remains mandatory: opening marks the lease claimed durably before returning the directory FD; owner cancellation after claim cannot remove GC protection.
- GC: treat only valid owned rows and claimed lease rows as roots. An ownerless or malformed reference must fail closed and prevent destructive collection rather than be ignored.

## Exact production callsites

`src/api/dispatch.rs` passes the peer identity to every caller-aware path:

```text
PIN / IMPORT_OCI(pin) / IMPORT_OCI_ARCHIVE(pin) -> pin_for or admit_oci_with_pin(..., owner)
UNPIN -> unpin_for
LEASE_CREATE -> lease_for(caller, grantee policy, ...)
LEASE_RELEASE -> release_for
OPEN_BLOB -> leased_for(caller, ...)
OPEN_PREPARED -> open_prepared_for(caller, ...)
```

The current visibility is: `Store::admit_oci` and `Store::admit_oci_pinned` are public wrappers, while `admit_oci_with_pin` is `pub(crate)`; `Store::import_oci_archive` and `import_oci_archive_pinned` are public wrappers, while `import_oci_archive_with_pin` is private; `Registry::pull` and `pull_pinned` are public wrappers, while `pull_with_pin` is private. Add caller-aware arguments to these existing paths rather than exposing new protocol bypasses. `src/oci/admission.rs::admit_oci_with_pin` adds `owner: Option<&PeerIdentity>` and writes the pin in the existing graph transaction. `src/oci/archive.rs::import_oci_archive_with_pin` forwards the same owner after importing the layout. `src/registry/pull.rs::pull_with_pin` forwards the owner on both the already-local fast path and the final verified admission path; the pin must never be created in a separate transaction after registry graph admission.

`src/prepare/mod.rs` changes its lease check to `leased_for(grantee, ...)` and later uses the claimed lease state. `src/gc` and startup recovery use explicit internal maintenance identity, never a socket caller, and must not adopt ownerless references.

## Authorization and boundedness

Role checks remain at the API boundary and inside the mutation transaction. Producer/admin may create pins; read-consumer may create leases for an authorized generic grantee; only the stored grantee may open a lease. Owner/grantee release and admin revoke follow the claimed-tree lifecycle described above. Operation tokens remain bound to UID/GID and lease create/release use the existing global 4096-entry journal window. Per-identity allocation quotas are **required future work in this full cutover**; they are not implemented by the current journal, which only enforces the global window.

## Qualification

Retain existing direct Store tests as trusted local producer coverage. Add public-boundary Linux qualification only after server role admission is enabled, using actual distinct numeric UIDs and `SO_PEERCRED` in isolated directories; no account creation or injectable identity seam is needed when a root-run harness uses `setresuid`/`setresgid`.

Required cases are: same owner same digest is idempotent; same ID with foreign owner fails; foreign unpin/release fails without mutation; legacy ownerless rows fail closed; delegated grantee can open while another UID cannot; claimed prepared leases remain GC roots after owner cancellation; and OCI/archive/registry pin creation records the same caller owner atomically with graph admission.
