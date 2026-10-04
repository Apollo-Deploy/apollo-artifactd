# Durable operation ownership

The daemon captures the kernel peer UID/GID before packet handling. Allocation
persists this generic identity with the operation reservation. The same redb
transaction that begins or completes an operation checks exact owner equality;
replay checks the owner before returning a recorded result. Current role policy
is separate from this identity. There is no caller-supplied owner field in the
wire request.

Internal restart reconciliation retains the historical owner while marking an
interrupted effect uncertain. Registry companion records must match the entire
operation record, including its owner. The existing 4096-record journal bound
and atomic retirement remain in effect.

Missing owner fields decode as legacy-unowned records. Startup journal audit
refuses these with an explicit migration error before binding the socket. It
does not infer an owner from the current connection or adopt the record on use.
This extends schema-3 record encoding; it is not a qualified legacy migration.
An explicit migration procedure remains a required cutover gate.

The server still admits only its own UID. Pins, leases, peer role policy,
allocation quotas across principals and delegated prepared-tree lifetime still
require implementation before opening admission to multiple service UIDs.

## Qualification scope

`tests/operation_owner_peer.rs` runs an unprivileged daemon and CLI under
isolated numeric UID 65534, with clients using GIDs 65534 and 65533. The kernel
GID check covers execution and terminal replay across a real SIGKILL restart.
Only the test launcher needs root; it copies binaries into an isolated fixture,
changes ownership only inside that fixture, and creates no Unix accounts.

`tests/operation_owner.rs` covers persisted owner mismatch and missing-owner
startup refusal. Its import fixture uses an independently reopened read-only FD
with the correct declared size. Ignoring only UID in the owner comparator lets
the import succeed, producing a real CAS receipt instead of the required
owner-mismatch rejection. This controlled regression is recorded in
`operation-owner-uid-pre-fix-arm64.txt`. Earlier attempts with an incorrect size
or an EOF cursor were discarded and are not qualification evidence.

ARM controlled regressions separately ignoring GID and removing the missing-owner
audit guard fail for unauthorized success and actual ownerless API availability,
respectively. Their logs are `operation-owner-gid-pre-fix-arm64.txt` and
`operation-owner-legacy-pre-fix-arm64.txt`. Earlier focused snapshots predate the
corrected import FD fixture. Final native logs and snapshots record the exact
qualified revision separately; they do not imply completed cross-UID admission
or full release qualification.

The final `operation-owner-native-arm64.txt` and
`operation-owner-native-x86_64.txt` logs pass both public persisted-state tests,
the explicit privileged launcher test with an unprivileged daemon, the ordinary
workspace/all-target tests, Clippy with warnings denied, and release all-target
builds. Both final snapshots match all 103 local source inputs. Ignored long
churn, registry, and fault campaigns are not covered by the ordinary suite.
