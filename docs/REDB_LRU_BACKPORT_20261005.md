# redb LRU queue repair — 2026-10-05

The bounded-memory gate remains open. Source-matched ARM Massif identified
2,155,008 bytes of live `VecDeque<u64>` queue metadata after 5,000 full blob
cycles. Reused offsets revive stale queue tokens in redb 4.3.0. The package
vendors that release with the exact upstream generation-tag repair from
commit f95d4609137481e6e40f04d9a103aa9541993267; only the LRU source file
differs. See [upstream provenance](../vendor/redb/ARTIFACTD_PATCH.md) and
[allocation evidence](RSS_CURRENT_RELEASE_20261005.md).

## Evidence

- The five upstream owner tests ran against original 4.3.0: queue boundedness
  and reinsertion eviction order failed for the intended reasons; three guards
  passed. All five pass with the exact backport on local macOS and native ARM.
- All 152 local redb unit tests pass. The local workspace has 50 passing test
  entries and one ignored entry; Linux-only test targets are excluded on macOS.
  The native ARM workspace records 68 passed, zero failed and 11 ignored.
  Helper entries and ignored external fixtures retain their existing scope.
- Local and native ARM strict workspace/all-target Clippy pass. ARM release
  daemon and runner builds pass; its 100-blob/20-OCI API smoke exited zero with
  clean inventory. ARM release workspace/all-target qualification also passed.
- Both ignored authenticated HTTPS registry fixtures pass on the repaired ARM
  snapshot: verified graph transfers and retries, then the daemon credential-FD,
  journal and prepared-facts path. The isolated loopback registry is fixture-owned;
  this is not a production endpoint test. The retained logs contain zero matches
  for the fixture's username, password or token. The [receipt](redb-lru-native-arm64-registry-receipt-20261005.json)
  records commands, return codes and log hashes. An initial harness attempt supplied
  the credential directory instead of its protected JSON file and was rejected;
  correcting the provider path required no implementation change.
- Production and fuzz dependency audit report zero vulnerabilities or warnings;
  cargo-deny and locked tree exit zero. The regenerated 294-component SBOM
  validates against the official schema, complete edges and current lock hash.
  Lock SHA-256: c06bbc3bfc2be2ce121d3bf0940903432c5ade61b44b40337c7a3a5179740f9b.
- The repaired ARM source snapshot contains 273 captured inputs with zero
  mismatches. Path/hash-map SHA-256:
  20df30b346439b40a8a93768705e265ccf85ee09d2adcdb572b0762c56ba3dfd.
  [Source inventory](redb-lru-native-build-inputs-20261005.json) retains paths.

The repaired ARM build completed all 100,000 blob operations and 20,000 OCI
lifecycles on `ssh apollo-node-01-internet`. Its [manifest](redb-lru-api-churn-arm64-manifest-20261005.json)
and [terminal summary](redb-lru-api-churn-arm64-summary-20261005.json) record
the private store, binary hashes, commands and inventories. Peak sampled daemon
RSS/HWM was 9,404 KiB before restart; the state file stayed at 3,538,944 bytes,
with 13 FDs and one thread. The final inventory has zero blob, prepared, temp,
GC, import, edge, pin, lease, registry and root rows; 4,066 operation rows are
within the 4,096 bound. Blob p50/p95/p99 latency was
18,735/47,568/296,066 microseconds and OCI latency was
36,980/108,811/740,926 microseconds. Observed throughput was 37,433 bytes/s;
host-load conditions make it unsuitable for cross-host comparison. This passes
the memory gate for this workload. Repaired x86 churn, registry throughput and
broader concurrency/load measurements remain open. The prior unpatched ARM
10,000 checkpoint was 15,236 KiB.

## x86 capacity failure

The first build accidentally used the old snapshot's `/tmp/target` location.
The host's separate 6.1 GB tmpfs filled even though its persistent filesystem
had about 4.98 GB free. Copying the failure log within that full tmpfs failed;
the subsequent retry's output redirection could not be written, so no
successful retry compilation occurred. Both build attempts exited nonzero;
further attempts were stopped. Logs were retained locally before removing
only the generated failed 210 MiB target created by this attempt. `/tmp`
then had 219 MB free. No qualification source or state store was removed.
A repaired x86 release and native memory/churn result remain required.

This repair is a mature-library backport, not a database replacement or
change to artifactd verification/durability. Unit tests, a smaller native
smoke, and audit tools do not grant independent release approval or close
the full caller/corruption/fault/fuzz/performance qualification gates.
