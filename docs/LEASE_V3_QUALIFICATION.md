# Protocol V3 qualification

Status: `APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`.

The current service uses exact kernel UID/GID policy roles, daemon-issued lease
identities, durable consumer claims, and descriptor-anchored socket binding.
OCI admission now returns verified manifest size and config/layer descriptor
facts for caller-owned receipts. Artifactd retains no build-specific identity.

Both native hosts passed `cargo test --locked --workspace --all-targets`.
Explicit privileged peer qualification is separate from the ordinary suite:
the ordinary run deliberately ignores the root-only process cases and their
consumer helper. The retained peer logs show the socket-directory regression,
two owner-isolation cases, and delegated consumer FD/restart/replay case.
Both hosts also passed strict workspace/all-targets Clippy and release binary
builds. Clippy initially rejected a boolean match; its conversion to `matches!`
retained the same policy patterns, followed by passing native role/lifecycle
cases on the final source.

The protocol crate alone additionally compiles under Rust 1.88.0 and 1.93.1.
Its declared MSRV is now 1.88, enabling the legacy caller workspace to use the
shared wire contract. The service workspace retains its separate MSRV.

On arm64, controlled restoration of absolute-path socket binding caused the
captured-directory bind test to fail because the socket appeared in the
replacement directory. Controlled authorization of creator cancellation for a
claimed lease caused the actual producer release to succeed unexpectedly,
failing the delegated-consumer test. Both repairs passed after restoration.
The independent scoped red-team report accepts the socket repair and withdraws
the lease-ID ABA concern after examining the actual V3 allocation contract.

Evidence:

- `lease-v3-{arm64,x86_64}-snapshot.json`: source and debug binary identities.
- `lease-v3-peer-{arm64,x86_64}.txt`: final native kernel-peer cases.
- `lease-v3-bind-claim-controls-arm64.txt`: deliberate regression controls.
- `lease-v3-counter-control-arm64.txt`: partial/zero local issuer metadata control.
- `RED_TEAM_ROLE_POLICY_V3.md`: independent scoped security review.

The snapshots follow final peer runs; the only later build metadata change was
the independently tested protocol MSRV declaration. The full ordinary suite
does not substitute for required long fuzz, representative layered-image churn,
claim/publication-window fault injection, registry delegation, resource quotas,
responsive concurrency, live caller cutover, Zig removal, or release approval.
Those gates remain open. The live BuildKit publisher/transports migration is
under implementation and is not qualified by this document.

## Protocol cancellation source qualification

The later `client-control-v3-{arm64,x86_64}-snapshot.json` snapshots match 111
current source/build inputs, including the callback-aware nonblocking client and
`tests/client_control.rs`. Both native ordinary workspace suites, strict
all-target Clippy, and release binary builds passed. The three privileged kernel
peer/claim cases were then selected explicitly from Cargo's current executable
artifacts and run with sudo on both hosts: operation UID/GID binding across
restart, persistent pin/lease ownership, and delegated claimed FD protection
through restart plus release replay. All passed. Outputs are retained in
`client-control-peer-{arm64,x86_64}.txt`.

The client cancellation test's independent ARM polling-callback bypass failed its
latency bound and restored source passed. These proofs cover wire cancellation,
final buffered responses, and the existing peer/claim cases. They do not prove
saturated listener connect behavior, cancellation of a real long daemon operation
followed by journal replay, full multi-role registry behavior, or all release
fault/churn/resource gates.
