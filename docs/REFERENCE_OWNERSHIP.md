# Reference ownership

Pin and lease records persist the kernel caller UID/GID. Direct in-process
Store methods derive the current process identity; the socket dispatcher uses
the accepted peer identity. Caller-aware methods are internal to the daemon.
Creation, same-ID retry, and removal authorize within the same redb write
transaction. A same-digest foreign reference cannot be adopted or removed.
Missing references remain harmless idempotent removals.

OCI graph admission creates an optional owned pin in the graph transaction.
Archive imports and registry pulls forward the same caller, including the
registry cache path. Leased blob and prepared-directory FD access check the
reference owner before exposing a descriptor.

Missing owner fields decode as legacy rows but require explicit migration.
GC verifies ownership presence while incrementally scanning roots and never
reaches deletion with an incomplete protection snapshot. Doctor also refuses
ownerless roots. No first caller or maintenance pass adopts an ownerless row.
This record extension does not qualify a migration of existing legacy state.

The server still admits only its own UID. Configured producer/consumer role
policy, per-principal quotas, explicit delegated grantees, and durable claimed
prepared-tree protection remain mandatory parts of the complete cutover.
Reference owner enforcement is a prerequisite to that work and does not grant
release approval.

## Qualification

Focused native public-boundary tests pass on ARM64 and x86_64. Both source
snapshots match 104 local inputs. The privileged launcher runs artifactd
unprivileged and uses same-UID/different-GID clients with freshly allocated
operation tokens. Foreign same-digest creation, removal, and blob-FD access
fail before and after SIGKILL restart; rightful retries preserve GC protection
and rightful removal permits collection. A late legacy root after 65 owned
roots makes bounded GC refuse with the specific migration error while bytes
remain verifiable.

ARM controlled regressions replacing exact owner equality with UID-only
comparison and removing the GC owner-presence guard fail for unauthorized
success and missing migration refusal respectively. Logs and matching
snapshots use the `reference-owner-focused-*` prefix; controlled logs use
`reference-owner-gid-pre-fix-arm64.txt` and
`reference-owner-legacy-pre-fix-arm64.txt`. Full native ordinary suites and Clippy pass on both hosts. Release builds also pass on both hosts. Both final snapshots match all 104
local source inputs; the `reference-owner-final-*` logs record ordinary
workspace/all-target tests, warnings-denied Clippy, and release/all-target
builds. Ignored long fault, registry, and churn campaigns are separate gates. Persisted UID-only tamper, foreign
prepared-FD access, and registry-specific peer-owner tests remain open.
