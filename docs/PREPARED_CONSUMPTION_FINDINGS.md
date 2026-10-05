# Prepared rootfs consumption findings

## Current Rust contract

Preparation publishes a digest-keyed directory below artifactd's private store. The
Rust implementation freezes directories at `0500`, regular files at `0400` or
`0500` according to the executable bit, rejects foreign ownership and writable
entries during verification, and records a deterministic tree digest. `OPEN_PREPARED`
passes the verified root directory through the artifactd Unix socket with
`SCM_RIGHTS`; it does not return an arbitrary filesystem path.

`PREPARE` currently returns `PreparedArtifactId`, `PreparedDigest`, source
`ManifestDigest`, `Platform`, size, and `format: artifactd-rootfs-v2`. The current
`OPEN_PREPARED` response returns the prepared artifact ID and the directory FD;
the consumer must obtain or carry the corresponding prepared facts from the
prepare receipt and verify the FD before use.

## Recommended boundary

Artifactd remains an unprivileged dedicated user with a private `0700` store.
Sandboxd consumes the transferred directory FD, validates it as the expected
prepared directory, and uses its existing privileged Linux mount boundary to bind
the FD into the jail. The target is then remounted read-only with `NODEV` and
`NOSUID`. This keeps artifactd from exposing a path and lets sandboxd use the
already documented `CAP_SYS_ADMIN`/`CAP_DAC_OVERRIDE` service capabilities.

The current sandboxd packaging runs as root and documents those capabilities in
`apollo-sandboxd/packaging/systemd/apollo-sandboxd.service`. A real native
qualification is still required: receive an actual FD from artifactd, bind it in
a private mount namespace, remount it read-only, verify the prepared digest and
platform, reject writes and unsafe entries, and release the lease only after the
consumer has stopped using the mount. This must run on both x86 (`tihan-apollo`)
and arm64 (`apollo-node-01`). Existing unit tests do not constitute this proof.

## Ownership and mutation boundary

A read-only bind mount protects the sandbox's mount view from writes by the
sandbox process. It does not prevent the owner of the source tree from changing
the underlying files; changes made by the artifactd UID can be visible through
the bind mount. Therefore the mount flag is not an integrity guarantee by itself.

The production threat boundary requires a dedicated artifactd UID, a distinct
authorized sandboxd caller, lease-gated FD transfer, and no customer access to
either service's host store. The current same-UID local socket authorization is
not sufficient to claim cross-UID integrity separation. The protocol should
carry the prepared digest, source manifest, platform, and format with the FD so
sandboxd can reject mismatched or stale receipts before mounting.

An ordinary unprivileged sandboxd with a different UID cannot recursively consume
the current `0500`/`0400` tree or mount it without additional host policy. A
shared consumer GID and a separately exported group-readable tree, or a
privileged mount helper, would be a different deployment contract and requires
explicit qualification. It is not silently implied by `SCM_RIGHTS`.

## Alternatives considered

`fs-verity` can authenticate regular-file contents, but it does not cover the
directory graph, ownership, modes, symlink policy, whiteouts, or the prepared
tree identity. It is therefore optional hardening, not a replacement for the
current descriptor-relative tree verification.

SquashFS or EROFS would still require a privileged mount boundary and a filesystem
image writer. Adding `mksquashfs`/`mkfs.erofs` subprocesses would violate
artifactd's no-shell/no-customer-command boundary, and the current mature Rust
dependencies do not provide a suitable writer. Keep the verified directory-FD
format until a separately qualified image writer and mount contract exist.

## Legacy Zig mapping

The Rust implementation preserves the legacy rootfs behavior: OCI layer ordering,
DiffID verification, gzip/zstd streaming extraction, whiteouts and opaque
whiteouts, bounded safe extraction, default symlink rejection, atomic
digest-named publication, immutable modes, and deterministic tree identity.

Zig path-based bundle consumption is replaced by the generic prepared ID/digest
and FD handoff. Business workload IDs and node ownership records remain outside
artifactd. The old marker/config path contract must not be reintroduced as a
trusted arbitrary path; its useful identity is represented by the protocol facts
and prepared receipt.
