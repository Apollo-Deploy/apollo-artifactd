# Direct BuildKit publisher cutover progress

Status: `APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`.

The live legacy publisher library now uses the service's shared V3 client,
without a Zig importer subprocess. Caller-owned build/source provenance stays
in its receipt. The archive is reopened read-only with `O_NOFOLLOW`, bound to
the operation intent by device/inode/size/SHA-256, and streamed through an FD.
Hashing uses bounded positional reads so it never moves the handoff FD offset.

Publication intent and receipt storage now use one captured private parent
directory capability. A per-receipt exclusive lock coalesces publishers; reads
are protected regular-file descriptors with bounded size, and writes use
exclusive UUID temporary files created at mode 0600, fsync, descriptor-relative
rename, and directory fsync. The durable intent remains after completion.

On both `tihan-apollo` and `apollo-node-01`, an isolated qualification workspace
built the actual publisher/protocol source with production pinned workspace
dependencies. A small public API harness published an independently generated
zero-layer OCI archive into a fresh unprivileged daemon, compared manifest and
config identity/size against independently hashed fixture bytes, repeated the
publication, removed the candidate archive, and reused the persisted receipt.
These checks passed. This was a caller library harness, not the full legacy API
workspace or the live BuildKit solve path. The publisher source additionally
passes its real workspace Linux-target `cargo check` without MSRV override.

An arm64 control replaced positional hashing with shared-offset clone/seek/read
hashing. The same public-boundary fixture then failed with `ImporterFailed` at
actual archive admission. Restoring positional reads passed again. The retained
smoke fixture is `scripts/native-publisher-smoke.py`, which accepts trusted
daemon and public harness executable paths. Authoring rationale: it protects a
real descriptor handoff, existing service tests cannot see the caller's offset
mutation, and it needs no test-only production export.

Independent review accepts the intent-filesystem and hashing repairs. Receipt
reuse still needs durable pin liveness/ownership confirmation, and descriptor
relationships require an independent caller verification step. Config reads
through a covering lease and transport solve/recovery migration are underway.
Live solve, ambiguous response/restart qualification, release configuration,
all callers, and removal of the old Zig implementation remain unfinished.
No production release or deployment is claimed by these checks.

## Receipt completion and live pin repair (2026-10-05)

The direct publisher now persists the admitted root digest and complete receipt in
its protected, locked intent before writing the external receipt. Receipt reuse
requires that completion record, compares the selected platform and every receipt
descriptor against a fresh artifactd `RESOLVE`, and durably records a new `PIN`
operation before re-establishing the exact caller-owned pin. An ambiguous pending
pin is replayed before a fresh liveness operation; a historical import receipt is
not treated as proof of present pin protection. Completed recovery does not need
the original archive. Archive and receipt-fact code are separate small modules.

Both native hosts rebuilt the actual publisher source and public integration
harness with the locked isolated qualification workspace. The extended
`scripts/native-publisher-smoke.py` passed on arm64 and x86_64: independently
hashed OCI descriptors, repeated publication, deliberate service-side `UNPIN`,
receipt reuse followed by GC, descriptor substitution with a recomputed caller
receipt checksum and altered completion record rejected as `ReceiptConflict`,
and receipt reuse after archive removal. These checks qualify this caller
boundary only. They do not qualify the full legacy workspace or BuildKit solve.

Transport solve/recovery source now uses the in-process publisher and its bounded,
lease-backed config FD reader rather than a publisher subprocess or direct CAS
path. Transport native compilation, cancellation during synchronous protocol
calls, production bootstrap configuration, and other caller/deployment migrations
remain required before cutover qualification. No old Zig implementation has been
removed on the strength of these scoped checks.

The independent ARM descriptor control bypassed only the comparison against
fresh resolved receipt facts. The forged descriptor was then accepted and the
public qualification failed for that intended reason. Restoring the comparison
and rebuilding passed. Its scoped evidence is recorded in the legacy repository
`docs/RED_TEAM_BUILDKIT_PUBLISHER.md`; this is not release approval.

Subsequent protocol/publisher source adds callback-controlled allocation and
request polling with monotonic deadlines, terminal socket error handling, and
buffered final response handling. Publisher callbacks also run during bounded
archive hashing and config lease/open/release calls. Archive opens are nonblocking
and still require a regular protected file. Linux-target publisher checks and
protocol Rust 1.88 strict Clippy passed. Native ordinary caller smoke passes with
the controlled client, but cancellation/replay under a deliberately slow service
and the blocking Unix-connect bound remain unqualified requirements. Prior
whole-service qualification snapshots do not prove these subsequent source
changes.

Bootstrap and the production environment example now configure
`APOLLO_ARTIFACTD_SOCKET` and `APOLLO_ARTIFACTD_UID` instead of publisher/importer
binaries or an artifact-store path. Production bootstrap requires the canonical
socket path, a non-root daemon UID, and matching socket ownership. Live solve and
config reads pass cancellation and wall-deadline callbacks. These transport and
bootstrap source edits pass formatting/metadata/diff checks; full native
control-plane/transport compilation and remaining staged verification/release
fixture migrations are still required. The macOS workspace build cannot validate
the Linux-only client.

## Native caller workspace compilation

Both native hosts built the actual 14-member legacy workspace with
`cargo check --locked -p apollo-transports -p apollo-control-plane`. An initial
compile found the missing `FileTypeExt` import in the new bootstrap socket
validation; it was repaired along with two unused imports left by publisher child
removal. The affected packages were explicitly cleaned in the isolated build
workspace before the final checks, and both final checks passed without warnings.

`buildkit-caller-inputs-20261005.json` fingerprints 887 legacy build inputs plus
five protocol inputs. All 892 hashes were independently checked against both
remote staged workspaces. Final build output is retained in
`buildkit-caller-{arm64,x86_64}-compile.txt`. These are compilation receipts,
not execution qualification for BuildKit, the control-plane, or the remaining
legacy fixtures. No legacy unit tests were added or run.

`tests/client_control.rs` exercises the public authenticated Unix protocol:
cancellation before transmission sends no packet/FD; cancellation after an
independent server observes the request preserves exact token/version/action and
FD bytes while returning within two seconds; a final response remains readable
when its server closes immediately. All three passed in the real daemon workspace
on both architectures. An independent ARM control removed only the callback from
the polling loop; the post-transmission case failed its two-second bound after
ten seconds, and restoring the callback passed. This proves response-wait
cancellation. The subsequent saturated-connect fixture passed on both architectures; removing
nonblocking connection mode in an ARM control failed its timing bound and restoring
it passed. See `client-backlog-native-qualification.txt`. Service-operation
cancellation with journal replay remains a separate required qualification case.

Current daemon ordinary qualification also passes on both architectures:
`cargo test --locked --workspace --all-targets`, strict workspace/all-target
Clippy, and release binary builds. Full logs are retained as
`client-control-{arm64,x86_64}-qualification.txt`. Both
`client-control-v3-*-snapshot.json` files match all 111 current source/build
inputs. Ordinary suite ignores remain explicit: privileged peer fixtures,
registry fixtures, and long churn are not implied by these green results.

## Verification caller source cutover

The legacy authenticated Build fixture now owns an isolated Rust daemon context,
passes its socket/service UID to the in-process publisher, and independently
materializes the receipt-bound BuildKit export archive rather than reading private
CAS paths. Trusted fixture executables are staged from validated captured FDs;
the worker has an owned systemd scope and bounded private output drain. The
source-admission shell fixture also selects an explicitly configured Rust daemon
socket/UID and no longer names importer/publisher subprocesses or artifact-store
paths. These caller edits pass Python compilation and shell syntax checks; real
BuildKit/control-plane execution with them is not yet qualified.

The isolated daemon helper passed native lifecycle checks on both requested hosts:
SO_PEERCRED-authenticated STATUS, producer blob import, configured numeric daemon
UID/GID, unlisted-peer rejection, protected executable staging, child reaping, and
owned state removal. Native checks discovered `/run` is mounted noexec on both
hosts; executable staging now uses a validated sticky `/var/tmp` parent.

The independent OCI archive helper hash
`538da08e95268eb41048827822b19befefab51e5711fa4546f1489bdf19f1742`
passed nine public cases on both architectures, including mutated content,
receipt mismatch, duplicate names, traversal, links/FIFO, and oversized GNU/PAX
extensions rejected before allocation. Evidence is retained in the legacy
workspace `docs/oci-archive-fixture-{arm64,x86_64}.txt`. The scoped independent
review is `docs/RED_TEAM_ARTIFACTD_FIXTURES.md`; it is not global release approval.
The archive helper has since changed to set every output directory/file mode
explicitly. Both native hosts passed materialization followed by retained-layout
verification under umask 022 without permission normalization, corruption/link/
foreign-entry rejection, and bounded rejection of a 4097-entry root directory.
Current archive helper SHA-256 is
`b6db6995e52791d6cd86605d736df52852030df8fd07ba3efb8ae38a5fd02abe`;
current retained-layout verifier SHA-256 is
`1de46a26881e47874262f1d2e598d4fde0cd57f4bbd5c4483835d15978296ac9`.
Current evidence is in legacy `docs/oci-layout-fixture-{arm64,x86_64}.txt`.
The Zig removal gate remains open.

## Persistent caller and release contract

The persistent verification caller now configures a stable Rust daemon managed by
an ownership-checked transient unit, retains CAS across restart, and verifies its
receipt-bound OCI layout before consumption. Both native hosts passed public
import/restart/inspect checks, staged-binary tamper rejection, missing established
CAS rejection, and foreign-unit collision rejection without stopping that unit.
Evidence is in `persistent-artifactd-fixture-{arm64,x86_64}.txt`. These helper checks
do not prove complete persistent customer build or Node execution.

Legacy BuildKit release/staging scripts no longer build, hash, or install the Zig
Artifact importer or standalone publisher process. Their five executor binaries
use the external artifactd service configured independently. Syntax validation
passed; complete release/staging execution, migration of the remaining proof
scripts, removal of previously installed obsolete binaries, and removal of all
legacy Zig Artifact source remain required.

The persistent helper stale-socket cleanup fix also passed on both hosts: a
foreign socket was rejected without deletion, the same helper retried successfully
after the control explicitly removed its owned socket, and FD count returned to
the four-FD baseline. Current helper SHA-256 is
`05a418732a99bff6ef9f455e6144a4668f398c683fc7dc26506873bfee05ad87`.

## Durable config lease ownership (2026-10-05)

The publisher config reader now requires a private caller intent path. It persists the daemon-issued creation token and its deterministically derived lease identity before `LEASE_CREATE`. Config access still uses a covering leased FD and independently verifies size and SHA-256. All useful-work failures, including cancellation, attempt bounded lease release; an ambiguous cleanup keeps its intent. Recovery releases by durable lease identity with a freshly persisted release token, so an expired old operation token cannot hide retained content. The private directory lock prevents concurrent reader/recovery effects for the same intent.

The actual transport publication supplies its private attempt path. Live terminal handling reconciles config leases before returning a terminal result, and restart recovery reconciles before exposing recovered cancellation/failure/publication. Whole-transport native compilation and integration qualification are still required for these new caller edits.

The focused ARM public-boundary fixture passed verified FD reading, cancellation releasing its lease, caller SIGKILL after an actual leased FD opened, daemon restart preserving that lease, repeated caller recovery, and subsequent GC removing all retained content. Native publisher production Clippy passed. Exact production source, protocol inputs, binary and isolated workspace lock provenance is `config-lease-arm64-snapshot-20261005.json`; the isolated qualification lock is not the full legacy production workspace lock. Its log is `config-lease-arm64-20261005.log`.

The independently isolated negative control omitted only the recovery release effect. It failed at `durable caller cleanup must release retention`, after caller SIGKILL and daemon restart, as recorded in `config-lease-arm64-negative-control-20261005.log`. Separate source and Cargo target directories prevented mutant reuse. This is native executable qualification, not a new legacy unit test or global red-team approval.

Test-audit rationale: the owning invariant is that a caller can relinquish a temporary config lease after lost responses or process death, without deleting in-use bytes. Existing service lease tests cannot observe the caller's pre-effect persistence or its terminal/restart cleanup. The fixture exercises public APIs and caller-produced records, never writes a lease receipt itself, and adds no production-only test seam. The SIGKILL point is selected by successful actual leased `OPEN_BLOB`, not private method-call count.

The same focused config-lease boundary subsequently passed on native x86 with strict publisher production Clippy. Exact inputs/binaries are retained in `config-lease-x86_64-snapshot-20261005.json`, and execution in `config-lease-x86_64-20261005.log`. This x86 run used the previously qualified V3 release daemon; the ARM run used the isolated later CAS-repair debug daemon. Neither result is a whole current release qualification. Fresh independent scoped review and whole production transport compilation remain pending.
