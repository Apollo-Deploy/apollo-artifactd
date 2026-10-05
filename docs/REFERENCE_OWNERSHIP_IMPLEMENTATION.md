# Reference ownership implementation

Artifactd now persists generic `PeerIdentity { uid, gid }` ownership for
references and operation journal records. Pins require an owner. Leases retain
owner, grantee, and lifecycle state. The API converts the optional protocol V3
grant identity to state data only after policy validation; a wire identity is
not authorization evidence.

Reference mutations compare the stored owner in the same redb transaction as
the mutation. Lease open authorizes the stored grantee and claims the lease
before descriptor publication. Available and claimed leases protect their
reachable OCI graph during GC. Ownerless or malformed legacy rows fail closed
rather than being adopted by the first caller.

Runtime role admission is configured by a service-owned policy file containing
`socket_gid` and generic producer/consumer/admin peers. `SO_PEERCRED` is
validated before request bytes and FD-bearing work. The default private runtime
still admits only the artifactd service identity.

Lease IDs come from daemon-issued operation tokens and cannot be caller-chosen
or reused. There is no current requirement for caller-selected IDs, an ID
reuse tombstone protocol, or automatic claim rollback. Uncertain claimed
leases remain protected until an explicit authorized completion/release.

Still-mandatory qualification includes native distinct-UID/GID integration,
delegated lease and grantee-only open, persistence across restart/SIGKILL,
immutable GC protection, per-principal quotas, and the complete registry and
cross-service deployment gates. Existing direct Store tests are not a
substitute for those native boundaries.
