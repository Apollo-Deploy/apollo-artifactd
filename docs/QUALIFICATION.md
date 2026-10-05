# Qualification evidence — updated 2026-10-05

**APOLLO_ARTIFACTD_PRODUCTION_PARTIAL. Independent release approval is blocked.**

This evidence covers the new standalone package and separately scoped caller probes. It does not qualify the existing deployment API or authorize production deployment. Both native hosts ran under their ordinary unprivileged accounts in isolated qualification directories; no existing daemon or artifact store was changed.

## Required gates

| Gate | Evidence and remaining work | Result |
|---|---|---|
| Migration inventory | [MIGRATION.md](MIGRATION.md) inventories the Zig library, importer, receipts, recovery and callers. Destination labels are not proof of completed porting. The old package, importer, registry probe, CI entry and importer-based restart wrapper are now removed. | Mapped; source cutover performed |
| Independent Rust build | Own protocol crate, Cargo.lock, transactional redb and documented C toolchain. Current-source production-bin builds and minimal `CAPABILITIES`/`STATUS` calls pass on native Linux x86_64 and ARM64 after the latest GC walk module split ([evidence](GC_WALK_MODULARIZATION_20261005.md)). Broader API, workspace, recovery and release qualification gates remain open. No Zig or Apollo sibling dependency in this package. | Production bins and minimal startup pass; full qualification incomplete |
| CAS | Streaming hash/size checks, private temporary files, fsync, atomic no-overwrite publication, duplicate/concurrent imports and corrupt-content rejection exercised. A direct Unix-socket smoke with the current release binaries on x86_64 and ARM64 verified FD import, wrong-digest rejection, duplicate identity, VERIFY and clean DOCTOR/STATUS results ([receipt](ARTIFACTD_NATIVE_SERVICE_SMOKE_20261005.md)). | Implemented; full failure campaign open |
| OCI | Local manifests/configs/layers/indexes, amd64/arm64 selection, descriptor/platform substitution tests, archive limits and graph-corruption rejection. The same direct service smoke imported a two-platform zero-layer index, resolved both platforms, prepared/opened the selected rootfs FD, then released references and collected all five graph blobs. It does not exercise layer extraction or hostile archives ([receipt](ARTIFACTD_NATIVE_SERVICE_SMOKE_20261005.md)). | Local subset passes |
| Registry | Authenticated HTTPS push/pull and protected credential FD tests passed earlier on both native architectures. Focused PUSH admission and cached-PULL authority regressions pass on both native hosts. PUSH manifest read-back now reuses the configured registry credentials; current release bins compile on both native hosts ([build evidence](REGISTRY_PUSH_AUTH_REUSE_20261005.md)), but the authenticated transfer was not rerun for this patch. The current redb-backport snapshot passes authenticated HTTPS store and daemon API paths on ARM64 against a private loopback fixture; its [receipt](redb-lru-native-arm64-registry-receipt-20261005.json) retains exact commands and secret-free log hashes. Current x86 authenticated transfer and the full fault/concurrency matrix remain open. | Implemented; qualification incomplete |
| Preparation | DiffID in both passes, image-wide decompression bounds, layer ordering, whiteouts, relative-link validation, deterministic hash and prepared recovery tests. Bounded GNU/PAX interpretation and conversion of safe rootfs-absolute symlink targets to relative targets are implemented and have separate native evidence. Absolute archive member paths, escaping links, hardlinks and devices are rejected. Owner-UID immutability is not enforced by read-only FDs. | Full hostile-image/fault/security gates open |
| Pins/leases/GC | Verified graph reachability, durable incremental root marking (at most 64 root records per call), an 8 MiB-equivalent metadata-read budget and 512 graph-work units per `gc_ready` call, epoch invalidation, leases, bounded deletion batches and ownership checks. The walk cursor is durable; the budgets reset per call. Focused GC/OCI and CAS-cursor regressions plus the full workspace suite have recorded native x86_64 and ARM64 passes in [GC_INCREMENTAL.md](GC_INCREMENTAL.md). These caps bound each call, but do not establish fairness, a fixed latency for one large graph, or behavior under sustained reference churn; broader fault/race and churn qualification remain open. | Incomplete bounded-work gate |
| Recovery | Actual SIGKILL during streamed import, CAS and prepared publication, and partial prepared GC passed on x86_64 and arm64; the preparation fixture consumed the recovered readonly FD. See [publication and GC interruption](PUBLICATION_CRASH_NATIVE_20261005.md). Capability-opened redb and graph classification-loss regression tests. Actual private-filesystem ENOSPC import/failure replay/restart/fresh retry passes on both architectures; see [focused disk-full evidence](DISK_FULL_IMPORT_NATIVE_20261005.md). Legacy SQLite migration and full structural corruption qualification remain open. | Full disk-full/power-loss/crash campaign missing |
| API | Linux SO_PEERCRED, bounded seqpacket/FD parsing, genuine daemon import/replay/concurrent-client tests. V3 exact UID/GID roles, delegated leases and persistent consumer claims have native kernel-peer evidence in [LEASE_V3_QUALIFICATION.md](LEASE_V3_QUALIFICATION.md); [client service-UID trust](CLIENT_IDENTITY.md) is separate. Serial processing can starve other clients. Bounded allocated-token replay rejects expired tokens before effects; complete concurrency qualification remains open. | Incomplete concurrency/idempotency gate |
| External integrations | Direct Rust BuildKit publisher and transport compilation plus focused publisher lifecycle evidence exist. Standalone buildd wires Artifactd into solve completion, persists import and optional registry-push identities, and replays pending publication on startup; its current release binaries compile natively on x86_64 and ARM64 ([caller evidence](BUILDD_V3_CLIENT.md)). The [Node ARM64 caller probe](NODE_CALLER_ARM64_QUALIFICATION_20261005.md) exercises the public Application boundary and prepared FD; the [x86_64 caller probe](NODE_CALLER_X86_QUALIFICATION_20261005.md) exercises resolve, preparation and restart replay. Separate sandboxd handoff fixtures pass on both architectures. Full production orchestration, VM guest execution, and runtime/restart fault qualification remain open. | Partial |
| Complete Zig removal | The legacy Artifact implementation, importer, registry probe, API CI entry, importer-based restart wrapper, and package-local Zig build outputs were removed. Active Rust publisher, buildd, and Node FFI callers use Artifactd. Historical receipts and evidence are retained as records/contracts, not executables. | Source/build cutover complete; release qualification remains separate |
| Performance/churn | Fresh repaired-source native public API campaigns completed 100,000 blob operations and 20,000 layered OCI lifecycles on x86_64 and ARM64. Both had exit 0, empty final content/reference/prepared/GC inventories, 4,066/4,096 operation rows, 13 FDs and one thread. Sampled peak RSS was 10,308 KiB on x86_64 and 9,460 KiB on ARM64; final state files were 3,538,944 bytes. [Source hashes, binary hashes, latency samples and receipts](REDB_LRU_API_CHURN_NATIVE_20261005.md). This qualifies bounded memory for this serial fixture workload only; registry throughput, concurrent-load/fairness and broader performance qualification remain open. | Native serial churn passes; broader performance open |
| Fuzz/property testing | Native ARM sanitizer builds cover all four targets. The recovery target mutates stored import metadata and checks foreign sentinels and failed-intent preservation; seven fixed recovery seeds pass and the deliberate missing-ownership guard mutant fails on the expected sentinel check. An ARM `protocol_oci` campaign was stopped after 480 seconds to protect host storage, so no 30-minute native fuzz target completed; see the [interrupted-run evidence](NATIVE_FUZZ_ARM64_INTERRUPTED_20261005.md). Native x86, full HTTP/auth/TLS fuzzing, arbitrary state-table corruption and the remaining property/fault matrix remain open. | Partial; campaigns open |
| Dependency security | Audit and deny pass, tree and CycloneDX SBOM generated. [DEPENDENCIES.md](DEPENDENCIES.md) records security-relevant acceptance limits. | Tool checks pass; review is conditional |
| Independent red team | The focused GC/OCI reachability follow-up approved that slice. Broader service findings remain in the [review record](RED_TEAM_IMPLEMENTATION_REVIEW_20261005.md); red-team work is paused at the user's direction. | Paused; service release gate open |

## Native execution

`apollo-node-01` is arm64; `tihan-apollo` is x86_64. Tests use `cargo test --locked --workspace --all-targets`; production builds use `cargo build --locked --release --all-targets`. Logs are `native-{architecture}-tests.txt` and `native-{architecture}-build.txt`. Test success does not imply all requested scenarios exist.

The churn example runs direct core calls, including verification and durable filesystem/database effects. Each host uses a fresh private store. Blob cycles use small unique contents; OCI cycles use valid zero-layer manifests/configs. These results establish that these fixtures finish at the requested counts; they do not establish daemon or registry load behavior. JSONL samples and `/usr/bin/time -v` output are retained. Final state inventories record empty tables/directories and SQLite size, but do not prove long-term journal replay semantics.

Measurements are generated from the retained native JSON/JSONL logs below. Cross-host throughput differences are not a controlled architecture comparison. Import and preparation use a 256 MiB uncompressed layer, with verification and fsync enabled. Registry pull/push throughput was not measured in this baseline campaign.

| Measurement | arm64 (`apollo-node-01`) | x86_64 (`tihan-apollo`) |
|---|---:|---:|
| Blob cycles | 100000 | 100000 |
| OCI cycles | 20000 | 20000 |
| Blob elapsed seconds | 304.941 | 395.733 |
| OCI elapsed seconds | 115.08 | 150.664 |
| Blob p50 / p95 / p99 (microseconds) | 2132 / 5942 / 7571 | 3807 / 5041 / 6116 |
| OCI p50 / p95 / p99 (microseconds) | 4385 / 8013 / 19974 | 7240 / 9822 / 12141 |
| Core final RSS / high water | 4600 kB / 7296 kB | 5560 kB / 8444 kB |
| Core final FDs / threads | 12 / 1 | 12 / 1 |
| Idle daemon RSS / FDs / threads | 4492 kB / 14 / 1 | 5416 kB / 14 / 1 |
| Idle CPU seconds (5-second sample) | 0.0 | 0.0 |
| Verified import MiB/s | 182.68 | 74.0 |
| Verified prepare MiB/s | 65.63 | 20.01 |

These are the final native reruns after the latest state/OCI guards. Each host passed the full current test suite (18 test entries, including the child-worker helper), release all-targets build, and core churn. The source/Cargo hashes in `native-{architecture}-snapshot.json` matched the production source bytes at baseline commit `3a42141`; subsequent feature evidence is recorded separately. Binary hashes, host uname and read-only final store inventories are retained; both inventories have zero rows and data-directory entries, `quick_check=ok`, an 86,016-byte main database and a zero-length WAL. The read-only inventory connection may create SQLite shared-memory sidecars. This is not the full required fault or daemon-journal campaign.

## Fuzz and dependency commands

The separate `fuzz/` workspace pins its own dependencies. Local Darwin sanitizer smoke runs used `cargo +nightly fuzz run protocol_oci`, `archive`, and `recovery`, with 30-second limits. Protocol smoke completed 300,109 executions; the final archive run with a valid OCI seed completed 5,508; recovery completed 5,777. Each completed without a sanitizer crash. Mutation coverage and short wall time are not a release fuzz qualification.

`cargo audit --json` was run for both production and fuzz lockfiles; both reports contain zero vulnerabilities and no warnings. `cargo deny check` passed advisories, bans, licenses and sources, with duplicate-version warnings. `cargo tree --locked` and a metadata-derived CycloneDX 1.5 SBOM are retained. `sbom-validation.txt` confirms official schema validation, unique component references and complete dependency-edge references for all 167 components. `cargo clippy --locked --workspace --all-targets -- -D warnings` passes. No dependency-audit exception was added to suppress an advisory.

## Subsequent implementation progress

[Producer-calculated imports](PRODUCER_IMPORT.md) add optional digest handling, durable unresolved/resolved intents and real SIGKILL recovery coverage after the baseline snapshot. Their native checks are separate from the baseline churn/performance logs. Registry-library hardening and capability-FD redb state conversion are implemented but are not release-qualified.

Current authenticated HTTPS integration tests passed on both native Linux architectures (`registry-redb-{arm64,x86_64}-tests.txt`): layered image push/pull, duplicate transfers, descriptor integrity, preparation, wrong references and authentication failures. The native workspace suites and warning-free clippy checks also passed (`redb-native-{arm64,x86_64}-tests.txt`, `redb-{arm64,x86_64}-clippy.txt`). These runs precede the subsequent prepared/blob GC cursor repairs and final upload-Location patch; those changes need their own native rerun. They do not qualify all registry fault/security/performance requirements.

`cargo-audit-redb-registry.json` and `cargo-audit-fuzz-redb-registry.json` record current lockfile vulnerability checks. `cargo-deny-redb-registry.txt` passes all checks after review of the certificate-data license retained in `licenses/`. The current 293-component SBOM includes the vendored OCI fork and redb; `sbom-validation-redb-registry.txt` validates its official schema, dependency references and lockfile hash. The 167-component baseline inventory is preserved as `sbom-baseline-3a42141.cdx.json`.

Latest integration checks are recorded in `registry-final-{arm64,x86_64}-tests.txt`: both the core HTTPS transfer test and the real daemon Unix-FD registry test pass. The API test verifies protected credential FD delivery, secret-free errors/journal records, pre-journal reference rejection and canonical `PreparedDigest` facts. It does not establish all registry response/fault cases. [Cursor recovery evidence](CURSOR_RECOVERY.md) records the subsequent bounded-page progress regressions and both native reruns. Current source/debug binary snapshots are separate from the baseline release binaries and performance logs.

A subsequent PUSH admission fix in `src/registry/push.rs` requires a persisted OCI root before registry authentication or transfer. The focused `push_rejects_unadmitted_manifest_before_credentials` test imports a schema-version-1 manifest as raw CAS content and supplies credentials for a different authority. It expects the local admission error, with no network request. A temporary local negative control without the guard failed as intended with `credential authority mismatch`, proving the request previously reached credential validation. Restoring the guard passed the focused test on macOS, x86_64 Linux, and ARM64 Linux in isolated CAS-repair checkouts. The ARM64 rerun used `ssh apollo-node-01-internet` after the original host alias timed out at banner exchange.

The cached-PULL path now checks a supplied credential provider's declared registry authority before returning an already verified local graph. The focused `pull_cache_rejects_mismatched_credential_authority` test passes on macOS, x86_64 Linux, and ARM64 Linux; a temporary local negative control without the check returned the cached graph. A cache hit still avoids the network, as documented in [PULL_PIN.md](PULL_PIN.md), so this check does not prove current remote authorization or a complete cross-principal content policy. Source hashes, checkout scope, commands, and both negative controls are in [focused native registry evidence](REGISTRY_ADMISSION_AUTHORITY_NATIVE_20261005.md).

After the credential-validation refactor, isolated loopback HTTPS fixtures on ARM64 and x86_64 each passed both the ignored authenticated registry store test and the daemon API credential-FD/journal/prepared-facts test with the changed production source. The tests ran sequentially against fixture-owned storage. The initial x86 fixture certificate carried a CA constraint and failed before the registry observed a request; a replacement server certificate without that constraint passed. The remote checkouts' lockfile differs only by the newer FFI workspace package entry, so these are scoped registry checks rather than a coherent current-workspace release qualification. See [focused native registry evidence](REGISTRY_ADMISSION_AUTHORITY_NATIVE_20261005.md).

The subsequent [current-workspace native snapshot](CURRENT_WORKSPACE_NATIVE_20261005.md) includes the FFI crate in the same `Cargo.lock` and matches 172 build/test input files across local, x86_64, and ARM64. Both Linux hosts passed the ordinary workspace suite, warning-free Clippy for all targets, and release builds for all targets. The ARM snapshot then passed the exact authenticated HTTPS store and daemon credential-FD tests sequentially against its owned loopback fixture; x86 authenticated fixture evidence remains separately scoped to the changed production source. These native checks do not close the caller cutover, churn/fault/performance, or release red-team gates.

Both [pre-repair current-workspace public API churn](API_CHURN_CURRENT_WORKSPACE_20261005.md) campaigns completed the requested counts and final-inventory checks but failed bounded memory. Fresh repaired-source x86_64 and ARM64 serial public API churn completed successfully; see [native repaired-source results](REDB_LRU_API_CHURN_NATIVE_20261005.md). Broader concurrency/load and registry performance qualification remain open. Earlier builds, tests, audits and red-team findings recorded in this document apply only to their captured source revisions; red-team work is currently stopped at the user's direction.

## Bounded daemon operation journal

[Operation journal qualification](OPERATION_JOURNAL.md) records the version-2
allocated-token contract, the actual pre-fix operation-4097 failure, and
100,000 public daemon mutation cycles on each native architecture. Expired
tokens reject effects, retained success/failure results replay, and interrupted
intents become explicit uncertain failures. Journal semantic auditing is
bounded to 4096 records. The measurements use debug builds and do not replace
the blob/OCI churn or release-performance gates above.

Real daemon SIGKILL checks cover partial blob imports and authenticated
registry pulls on ARM and x86. Both native workspace suites and
warning-free clippy checks passed at that journal revision; its source
snapshots identify that earlier revision.
Additional native checks and source snapshots are
retained in the journal evidence files. [Independent round 5](RED_TEAM_ROUND_5.md)
closes permanent journal exhaustion while retaining the remaining release
findings. `journal-protocol-fuzz.txt` records a 30-second sanitized macOS
protocol/OCI/token-parser campaign: 268,854 inputs and no finding. This is
targeted fuzz evidence, not native Linux or full recovery/registry fuzz proof.

## Release decision

[GC descriptor-edge repair](GC_DESCRIPTOR_EDGES.md) removes the global
edge-table rewrite performed for every collected blob. Exact leaf reimport
restores surviving manifest usability without changing its descriptor graph;
missing children still reject pinning and resolution. The regression fails
before the repair and passes afterward on both native architectures, followed
by passing normal workspace suites and warning-free clippy. The separate
`oci-gc-edge-snapshot-{arm64,x86_64}.json` files identify the 71 source and
qualification files from that earlier repair. They precede incremental marking.

[Incremental GC evidence](GC_INCREMENTAL.md) records the subsequent durable mark phases, reference epochs, bounded root scans and native mutation/recovery checks. These replace global GC reachability scans but do not prove the entire churn, latency or fault gate.

At commit `29f069f`, both native hosts completed a fresh 100,000-blob and
20,000-zero-layer-OCI churn run with incremental marking enabled. Logs are
`gc-churn-current-{arm64,x86_64}.txt`. Each ended with zero blobs/bytes,
11 FDs and one thread. Sampled process peak RSS was 6528 kB on ARM and
7432 kB on x86. The external time command includes compilation and must not
be interpreted as service-only peak memory or CPU. These runs precede the
Doctor integrity and two-phase commit changes; they do not qualify those
changes or the entire required state-growth/performance matrix.

[Doctor integrity policy](DOCTOR_INTEGRITY.md) records page checking,
quarantine, historical maintenance receipts and two-phase Immediate commits.
[Independent Doctor review](RED_TEAM_DOCTOR.md) found no remaining material
defect in that implementation, while retaining the broader fault campaign
and global release gates. `doctor-native-{arm64,x86_64}.txt` records passing
final exact daemon corruption checks, normal workspace suites, warning-free
clippy and real SIGKILL import/prepared-GC recovery. Both debug snapshots
match 79 current source/qualification files. These checks precede the new
two-phase-commit churn measurement and remain separate from earlier churn.

The two-phase-commit churn at `30530f6` completed successfully on both hosts:
100,000 blob operations and 20,000 zero-layer OCI lifecycles, with restart
Doctor clean and zero rows in all artifact/import/reference/prepared/GC/journal
tables. Each retains only three metadata rows. Database samples stabilized
at 126,976 bytes during blob churn and 90,112 bytes during OCI churn;
both finish at 90,112 bytes, 11 FDs and one thread. Final sampled RSS was
5480 kB on ARM and 6292 kB on x86. `doctor-churn-{arm64,x86_64}.txt` includes
fixture, lockfile and release-runner hashes and process-only time measurements.
These runs precede the tar changes and do not qualify registry/daemon load or
representative layered churn.

[Bounded tar extensions](TAR_EXTENSIONS.md) records standard GNU/PAX handling,
malformed/duplicate metadata rejection and repeated extraction verification.
Both native final normal suites, warning-free Clippy and release all-targets
builds passed for the parser changes. Their 97-file source snapshots match
the production and qualification source at that run; the later registry
benchmark receipt-check correction changes only the example. These runs are separate
from the completed Doctor churn and exclude ignored crash campaigns. Dependency audit/deny/tree and the 293-component SBOM have been
regenerated for the vendored tar lockfile.

[Registry performance](REGISTRY_PERFORMANCE.md) records eight verified 32 MiB
HTTPS push/pull samples per host. Current 256 MiB verified import/prepare
results are `tar-performance-{arm64,x86_64}.txt`: ARM 197.98/52.22 MiB/s,
x86 73.30/18.72 MiB/s. Idle daemon RSS was 5152/5976 KiB respectively,
13 FDs and one thread, with zero observed CPU ticks over five seconds.
ARM initially exceeded the Unix socket path length in the measurement
harness; a private short runtime directory corrected that probe, with the
successful result retained in `tar-idle-arm64.txt`. These are scoped fixture
measurements and do not close concurrent daemon or long-run registry gates.
The subsequent `tar-crash-{arm64,x86_64}.txt` checks passed actual SIGKILL
recovery for interrupted imports and partial prepared-tree deletion on both
hosts; they do not replace the full crash/power-loss campaign.

[Archive path compatibility](ARCHIVE_PATHS.md) subsequently enables standard
OCI layout GNU/PAX handling and canonical current-directory members in both
import and rootfs preparation. Its focused suites, native normal suites,
Clippy and release all-targets builds passed on both architectures for their
recorded snapshots. The later caller, CAS-repair and recovery evidence is
scoped to separately captured revisions and binaries; it does not establish a
single coherent release build of the current workspace. Separate ignored
registry and real SIGKILL checks passed on both hosts for their captured
revisions. They do not complete the power-loss, registry fault or concurrency
campaign.
[Caller cutover findings](CALLER_CUTOVER_FINDINGS.md) identify the actual
producer paths and mandatory token, peer authorization and pin/GC changes
needed before integration. [Prepared consumption findings](PREPARED_CONSUMPTION_FINDINGS.md) clarify
the required UID and privileged sandbox mount boundary. Read-only bind mounts
do not protect the underlying tree from its store owner; current same-UID
authentication is insufficient for the intended service separation.
The findings remain applicable to the final service boundary. The ARM64 Node
and sandboxd probes above establish narrow caller and consumption behavior,
while end-to-end workload and cross-architecture qualification remain open.

The Rust service now replaces the old Zig Artifact source and active callers use its protocol. Production completion still requires the open registry/credential, bounded GC/concurrency, durable replay, native fault/performance, and dependency-review campaigns, followed by independent red-team approval. Red-team work is paused at the user's direction, so `RED_TEAM_RELEASE_APPROVED` and `APOLLO_ARTIFACTD_PRODUCTION_COMPLETE` have not been earned.

Retained command logs may have trailing whitespace normalized; test results and diagnostics are preserved.

## Atomic OCI admission with a caller pin

[Admission pin qualification](ADMISSION_PIN.md) records the optional generic
pin on OCI graph/archive admission. Graph roots/edges and pin ownership commit
in one redb transaction. The real daemon regression covers FD delivery,
same-digest admission under a fresh token, conflicting pin ownership, GC,
restart/replay, retained pin protection after restart, and release/collection.
The controlled old effect fails because GC collects content after admission.
This closes the artifactd import-to-pin gap; actual buildd publication and
caller registry-pull lifecycle integration still requires qualification.

## Registry pulls with atomic GC protection

[Pull pin qualification](PULL_PIN.md) covers optional pin admission through
both authenticated HTTPS transfer and verified local-cache reuse. The public
daemon test checks a new cache-hit pin independently by releasing the original
pin and collecting before preparation. Full registry fault/concurrency and
caller lifecycle cutover gates remain open.

[Durable operation ownership](OPERATION_OWNERSHIP.md) records exact kernel
UID/GID token binding, ownerless startup refusal, controlled negative cases,
and final native ordinary-suite/Clippy/release qualification on both hosts.
This does not implement pin/lease ownership, delegated consumer lifetime,
per-principal quotas, or server admission for distinct service UIDs.

[Reference ownership qualification](REFERENCE_OWNERSHIP.md) records owned
transactional pin/lease creation, removal, and FD access, bounded GC refusal
on ownerless roots, real kernel GID tests across restart, controlled
regressions, and native ordinary-suite/Clippy/release builds on both hosts.
Delegated claims, consumer lifetime, configured roles, and migration of legacy
references remain mandatory and unqualified.
