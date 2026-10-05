# Caller ownership and service roles

Artifactd protocol V3 binds journal operations and persisted references to the
kernel `(uid,gid)` from `SO_PEERCRED`. The server admits configured generic
roles: `producer`, `consumer`, and `admin`. Policy is checked before dispatch
and before journaled lease intent; state transactions repeat ownership checks.
No Apollo business IDs are used.

A policy file names the socket group and at most 64 configured peers. The
service peer must be an explicit administrator. A producer can import, prepare,
pin, and use registry operations. A consumer is limited to observations and
its authorized leases. An administrator performs maintenance. The private
runtime remains the default when no policy is supplied; an explicit policy uses
service-owned `0710` runtime and `0660` socket permissions.

Operations retain their daemon-issued IDs and exact owner. References retain
owner and, for leases, grantee and lifecycle state. Lease IDs are derived from
operation tokens; callers cannot select or reuse them. Claimed leases remain
GC roots until authorized completion. Legacy ownerless state fails closed and
is never silently assigned to the current caller.

Native Linux qualification remains required for real distinct UID/GID policy
admission, delegated leases, grantee-only FD access, restart/crash recovery,
GC safety, and resource quotas. This document does not claim that gate is
complete.
