# artifactd-client-ffi

This crate is the Rust-owned caller boundary for non-Rust Apollo runtimes.
It wraps `artifactd_protocol::client::Client`; it does not parse the wire
format, implement OCI admission, registry behavior, or access the artifact
store. The one OCI-specific helper uses the mature `oci-spec` crate to decode
a bounded config blob that artifactd has already verified and exposed through
an authenticated lease. It only returns typed runtime facts; it does not make
storage, graph, digest, registry, or extraction decisions.

Every mutating or lease-backed operation accepts the caller's persisted
`OperationId`. `artifactd_operation_allocate` is the only allocator. Returned
facts are opaque handles whose JSON is bounded by the daemon protocol; callers
should use the typed digest and size accessors where available. Returned file
descriptors are owned by the caller and must be closed with
`artifactd_fd_close` (or the platform equivalent).

`artifactd_pull_with_credentials` duplicates the supplied credential FD only
for the request and leaves the caller's descriptor open. Credential bytes are
never copied into artifactd state or logged by this library.

The ABI is Linux-only because the protocol's authenticated transport requires
Unix `SO_PEERCRED` and `SCM_RIGHTS`.
