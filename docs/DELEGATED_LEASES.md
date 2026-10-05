# Delegated leases (protocol V3)

Artifactd leases are generic capability records. `LEASE_CREATE` accepts an
optional protocol `PeerIdentity { uid, gid }` grantee. If omitted, the caller
is both owner and grantee. A producer may delegate only to a configured
consumer or administrator; a consumer may delegate only to itself. The daemon
resolves and validates the requested identity before it writes a journal
intent. A wire UID/GID never grants access by itself.

Lease IDs are derived from daemon-issued operation tokens. Callers cannot
choose or reuse them. Replaying the same token returns its recorded result;
changing its digest or grantee is a conflict. The lease record retains the
digest, owner, grantee, and lifecycle state. Claiming a lease is durable before
an FD is published, and claimed leases remain GC roots until the authorized
completion/release is durable. Recovery does not infer that an FD was closed
from a process exit or an uncertain response.

Pins have an owner but no lease grantee. Ownerless or malformed references are
legacy state and fail closed; they are never adopted by the first caller.

## Runtime policy

An explicit policy file is JSON and is validated before the store opens:

```json
{
  "socket_gid": 2000,
  "peers": [
    {"uid": 1000, "gid": 1000, "role": "admin"},
    {"uid": 1100, "gid": 1100, "role": "producer"},
    {"uid": 1200, "gid": 1200, "role": "consumer"}
  ]
}
```

The policy is service-owned, regular, single-link, non-group-writable, and
limited to 64 KiB. The runtime directory is service-owned `0710` with the
configured group, and its socket is `0660`; without a policy the existing
private `0700`/`0600` runtime remains in force. `SO_PEERCRED` is checked before
request bytes or descriptor-bearing actions are accepted. The store, CAS, and
prepared trees remain private to artifactd.

## Qualification status

Native Linux x86_64 (`tihan-apollo`) and arm64 (`apollo-node-01`) passed the
explicit root-run public API cases `kernel_peer_gid_binds_operation_owner_across_restart`,
`kernel_peer_gid_binds_persistent_pin_and_lease_owners`, and
`delegated_claim_survives_restart_and_release_replay_cannot_rebind`. The last
case uses separate producer, consumer, and service UID/GID pairs, holds a real
consumer FD through daemon SIGKILL/restart, verifies retained GC protection,
and checks create/release replay cannot rebind protection to a fresh lease.
Denied role and wire-grantee requests reuse their token in a subsequently valid
request, proving denial happened before journal intent.

On arm64, deliberately allowing creator release of a claimed consumer lease
made the public lifecycle test fail at the producer-release denial. Restoring
grantee-only release passed. The runtime bind regression similarly failed when
restoring absolute-path binding and passed after descriptor anchoring.
See [controlled regression output](lease-v3-bind-claim-controls-arm64.txt).

These are scoped lifecycle and runtime checks. Crash during the exact claim/FD
publication window, per-principal quota, registry delegation, and the full
cross-service deployment gate remain unqualified.
