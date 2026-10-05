# Lease lifecycle (protocol V3)

Artifactd leases are daemon-issued capabilities. `LEASE_CREATE` carries an
optional generic `PeerIdentity` grantee; omission makes the caller the grantee.
The API resolves and authorizes that identity before the journal intent is
persisted. Lease IDs are derived from operation tokens, so callers cannot
choose, recycle, or retarget an ID.

A lease records its digest, owner, grantee, and lifecycle state. `OPEN_BLOB`
and `OPEN_PREPARED` authorize the stored grantee and durably claim the lease
before publishing a descriptor. Available and claimed leases are GC roots.
Recovery preserves a claimed lease after an uncertain response; it does not
infer that a returned descriptor was closed. Completion/release is an explicit
authorized transition, after which the root may be collected in a later GC
cycle.

Ownerless or malformed legacy references fail closed. There is no caller-ID
selection, ID-reuse tombstone protocol, or automatic claim rollback promised
by the current implementation. Those older proposals must not be used as
compatibility requirements.

The remaining gate is native Linux qualification with actual distinct peer
credentials, SIGKILL/restart recovery, grantee-only access, and GC protection.
See [DELEGATED_LEASES.md](DELEGATED_LEASES.md).
