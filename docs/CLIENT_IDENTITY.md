# Protocol client service identity

The Linux protocol `Client` accepts the Unix socket path and the trusted server
UID. It checks kernel `SO_PEERCRED` immediately after connecting, before cloning
an input FD or sending any request bytes. Both mutation token allocation and
operation execution use this same check. CLI `--server-uid` selects that UID;
omitting it retains same-UID trust. The configured value identifies a trusted
Unix principal, not a specific process or executable.

This is a prerequisite for dedicated artifactd store ownership. It does not
widen server admission: the daemon still accepts only its own UID. Cross-UID
service access requires persisted operation/reference ownership, explicit peer
roles, authorized lease recipients, credential-FD ownership checks against the
caller, and a separately protected socket directory. Those production gates
remain open; making the socket accessible alone would expose currently unowned
operation tokens, pins and leases.

## Test ownership

`tests/client_identity.rs` owns the public client/CLI boundary contract. A real
Linux seqpacket listener with the wrong configured UID must receive neither
request bytes nor an input FD. A valid listener returns a bound response. Token
allocation and CLI automatic allocation must also reject the wrong UID.
Existing daemon tests exercise same-UID server behavior, not selection of a
foreign service UID. These cases use production client APIs and require no
production test seam.

The ignored `privileged_cross_uid_cli_identity_boundary` case runs only the
test binary as root. It creates an isolated root-owned socket fixture and
executes artifactctl as UID/GID 65534. Explicit trust of UID 0 must succeed;
the default trust of UID 65534 must reject the endpoint before sending. This
proves real kernel cross-UID client authentication, not cross-UID daemon policy.
No production daemon, account, registry or store is changed.

Native logs are `client-identity-native-arm64.txt` and
`client-identity-native-x86_64.txt`. Each records the focused ordinary cases,
explicit privileged kernel cross-UID case, full ordinary workspace/all-targets
suite, Clippy with warnings denied, and release workspace/all-targets build.
Long-running ignored registry/fault/churn cases are not implied by that suite.
Matching source/build snapshots are `client-identity-final-*-snapshot.json`.
The controlled regression is `client-identity-pre-fix-arm64.txt`.
The controlled regression replaces the configured-UID comparator with the old
caller-UID comparator, leaving transport and response behavior intact. The
wrong-UID FD case must fail because the client accepts an unauthorized endpoint
and receives its valid response, not due to another guard or a transport error.

Full release approval remains unproven.
