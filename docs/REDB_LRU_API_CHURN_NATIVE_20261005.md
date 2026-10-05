# Repaired redb public API churn — native Linux — 2026-10-05

The repaired source completed fresh, isolated public Unix-socket API campaigns
on `tihan-apollo` (x86_64) and `apollo-node-01-internet` (aarch64). Both
campaigns ran 100,000 blob operations and 20,000 layered OCI lifecycles,
including restart, against private stores. Neither ran a Cargo test target.

Both hosts built the same 234 build-input files (`Cargo.toml`, `Cargo.lock`,
`src/`, `crates/`, `vendor/`, and `examples/`), aggregate SHA-256
`21006eb7364bd95f42d44ac7fbfe7d0ce03859a487b8ac3811e1d19624821946`.
The native release command was:

```sh
CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_INCREMENTAL=0 \
CARGO_TARGET_DIR=/dev/shm/<private-run>/target \
cargo build --locked --offline --release --bins --example api_churn
```

The daemon and churn-runner binary hashes, plus the exact source manifest, are
retained beside each raw JSONL receipt:
[x86_64 manifest](api-churn-redb-lru-x86_64-20261005.manifest),
[x86_64 binaries](api-churn-redb-lru-x86_64-20261005.binaries.sha256),
[x86_64 samples](api-churn-redb-lru-x86_64-20261005.jsonl),
[ARM64 manifest](api-churn-redb-lru-arm64-20261005.manifest),
[ARM64 binaries](api-churn-redb-lru-arm64-20261005.binaries.sha256), and
[ARM64 samples](api-churn-redb-lru-arm64-20261005.jsonl).

| Observation | x86_64 | aarch64 |
| --- | ---: | ---: |
| Runner result | exit 0 | exit 0 |
| Blob operations / OCI lifecycles | 100,000 / 20,000 | 100,000 / 20,000 |
| Elapsed workload time | 797.2 s | 285.1 s |
| Blob cycle p50 / p95 / p99 | 4,184 / 17,739 / 20,350 µs | 1,642 / 6,380 / 6,515 µs |
| OCI cycle p50 / p95 / p99 | 8,282 / 25,608 / 29,068 µs | 2,660 / 8,683 / 8,818 µs |
| Highest sampled daemon RSS/HWM before restart | 10,308 KiB | 9,460 KiB |
| Final RSS after restart | 8,808 KiB | 7,740 KiB |
| FDs / threads | 13 / 1 | 13 / 1 |
| Final state file | 3,538,944 bytes | 3,538,944 bytes |
| Operation rows / configured cap | 4,066 / 4,096 | 4,066 / 4,096 |
| Final blobs, edges, imports, pins, leases, prepared, GC, registry rows | all zero | all zero |
| Final blob/temp/prepared entries; store entries | 0 / 0 / 0; 5 | 0 / 0 / 0; 5 |

No host disk guard fired. The RSS remained bounded during this serial fixture
workload, in contrast to the pre-repair runs. The measured byte-rate is fixture
throughput, not a calibrated import, registry, or service-capacity benchmark.
These campaigns do not measure concurrent clients, single-flight contention,
registry transfers, sustained reference churn, or fairness under load; those
qualification gates remain open. The service release gate also remains open.

No unit tests were created or run for this continuation. Red-team work remains
stopped at the user's direction.
