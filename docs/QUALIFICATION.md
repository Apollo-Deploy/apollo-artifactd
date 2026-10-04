# Qualification evidence — 2026-10-04

**APOLLO_ARTIFACTD_PRODUCTION_PARTIAL. Independent release approval is blocked.**

This evidence covers the new standalone package. It does not qualify the existing deployment API, remove Zig callers, or authorize production deployment. Both native hosts ran under their ordinary unprivileged accounts in isolated qualification directories; no existing daemon or artifact store was changed.

## Required gates

| Gate | Evidence and remaining work | Result |
|---|---|---|
| Migration inventory | [MIGRATION.md](MIGRATION.md) inventories the Zig library, importer, receipts, recovery and callers. Destination labels are not proof of completed porting. | Mapped; incomplete cutover |
| Independent Rust build | Own protocol crate, Cargo.lock, transactional redb and documented C toolchain; release builds on Linux arm64 and x86_64. No Zig or Apollo sibling dependency in this package. | Pass for this package |
| CAS | Streaming hash/size checks, private temporary files, fsync, atomic no-overwrite publication, duplicate/concurrent imports and corrupt-content rejection exercised. | Implemented; full failure campaign open |
| OCI | Local manifests/configs/layers/indexes, amd64/arm64 selection, descriptor/platform substitution tests, archive limits and graph-corruption rejection. | Local subset passes |
| Registry | Authenticated HTTPS push/pull and protected credential FD tests pass on both native architectures; complete registry fault and concurrency qualification remains open; scoped throughput measurements are recorded separately. | Implemented; qualification incomplete |
| Preparation | DiffID in both passes, image-wide decompression bounds, layer ordering, whiteouts, relative-link validation, deterministic hash and prepared recovery tests. Bounded GNU/PAX interpretation is implemented; final native extended-header qualification is recorded separately. Hardlinks and absolute links remain rejected. Owner-UID immutability is not enforced by read-only FDs. | Incomplete required portability/security |
| Pins/leases/GC | Verified graph reachability, durable incremental root marking, epoch invalidation, leases, bounded deletion batches and ownership checks. Native root-mutation and prepared-intent recovery tests pass; churn and fault/race qualification remain open. | Incomplete bounded-work gate |
| Recovery | Actual SIGKILL during streamed import plus synthetic persisted snapshots for CAS publication, prepared publication and post-unlink GC. Capability-opened redb and graph classification-loss regression tests. Legacy SQLite migration and full structural corruption qualification remain open. | Full disk-full/power-loss/crash campaign missing |
| API | Linux SO_PEERCRED, bounded seqpacket/FD parsing, genuine daemon import/replay/concurrent-client tests. [Explicit client service-UID trust](CLIENT_IDENTITY.md) is qualified separately; server cross-UID authorization remains unimplemented. Serial processing can starve other clients. Version-2 bounded allocated-token replay rejects expired tokens before effects; complete concurrency qualification remains open. | Incomplete concurrency/idempotency gate |
| External integrations | Protocol client is independently usable; BuildKit/buildd/sandboxd/Node/release callers are not migrated. | Missing |
| Complete Zig removal | Old Artifact implementation, importer and production launch/build paths remain in the original repository. | Missing |
| Performance/churn | Both native hosts completed 100k blob cycles and 20k zero-layer OCI cycles. Throughput, idle CPU/RSS, thread/FD counts and core p50/p95/p99 recorded below. Daemon operation-journal churn is recorded separately; registry throughput is now measured below. Representative layered-image churn remains missing. | Partial measurements |
| Fuzz/property testing | Property tests and three short local sanitizer fuzz smoke runs. No native long campaign or registry-response target. The full required property matrix is not proved. | Partial |
| Dependency security | Audit and deny pass, tree and CycloneDX SBOM generated. [DEPENDENCIES.md](DEPENDENCIES.md) records security-relevant acceptance limits. | Tool checks pass; review is conditional |
| Independent red team | [round 1](RED_TEAM_ROUND_1.md), [round 2](RED_TEAM_ROUND_2.md), subsequent review when present. Critical/high mandatory requirements remain open. | Blocked |

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
Clippy and release all-targets builds pass on both architectures; both
97-file final snapshots match current production and qualification source.
The separate ignored registry and real SIGKILL checks also passed on both
hosts for this production source. They remain scoped coverage, not the full
power-loss, registry fault or concurrency campaign.
[Caller cutover findings](CALLER_CUTOVER_FINDINGS.md) identify the actual
producer paths and mandatory token, peer authorization and pin/GC changes
needed before integration. [Prepared consumption findings](PREPARED_CONSUMPTION_FINDINGS.md) clarify
the required UID and privileged sandbox mount boundary. Read-only bind mounts
do not protect the underlying tree from its store owner; current same-UID
authentication is insufficient for the intended service separation.
These are findings, not completed caller migration or consumption qualification.

The package must not replace the existing Artifact implementation yet. Registry, secure credentials, all caller cutovers, old Zig removal, bounded global GC/concurrency, durable replay guarantees and the remaining qualification campaigns are mandatory work in this request. They are not deferred to a future version. `RED_TEAM_RELEASE_APPROVED` and `APOLLO_ARTIFACTD_PRODUCTION_COMPLETE` have not been earned.

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
