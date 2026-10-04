# Registry performance qualification

`registry_throughput` is an operator-run qualification probe for an isolated
authenticated HTTPS OCI registry. It creates one private source and target
store per sample beneath the administrator-provided directory, and leaves
those directories for inspection. It never removes paths it did not create.

The default is eight samples of a unique 32 MiB uncompressed tar layer. Set
`ARTIFACTD_REGISTRY_ROUNDS` for up to 100 samples. Every push and pull is
verified by artifactd, including digest, graph, and durable CAS checks. Pulls
use the returned immutable manifest digest; push and pull timings therefore
include real registry transfers rather than warm local deduplication.

Run on a private fixture with a protected credential-provider file:

```sh
ARTIFACTD_TEST_REGISTRY_REFERENCE='registry.example/apollo/qualification:latest' \
ARTIFACTD_TEST_REGISTRY_CREDENTIALS=/private/path/credentials.json \
cargo run --release --example registry_throughput -- /private/path/artifactd-registry-bench
```

The credential file is opened through artifactd's descriptor checks and its
contents are never printed or placed in artifact state. The JSON result reports
p50/p95/p99 latency, aggregate transferred throughput, fixture scope, and
Linux process RSS/thread/FD observations when `/proc` is available. CPU is
reported as cumulative Linux user+system scheduler ticks; RSS and FD values
include baseline, final, and per-round samples for peak inspection. With the
default eight samples, percentile tails are descriptive rather than stable
capacity estimates; increase the round count for a qualification run.

## Native results

Both isolated authenticated HTTPS fixtures completed eight samples with
verification and fsync enabled. Retained `registry-performance-{arm64,x86_64}.txt`
logs identify source, lockfile and release-runner hashes; compilation is outside
the external time measurement.

| Measurement | arm64 | x86_64 |
|---|---:|---:|
| Push MiB/s | 12.677 | 7.533 |
| Pull MiB/s | 37.566 | 19.447 |
| Push p50 / p95 / p99 seconds | 2.527 / 2.608 / 2.608 | 4.231 / 4.473 / 4.473 |
| Pull p50 / p95 / p99 seconds | 0.857 / 0.876 / 0.876 | 1.646 / 1.685 / 1.685 |
| Sampled peak RSS KiB | 26772 | 28332 |
| Sampled peak FDs | 22 | 22 |
| Final threads | 5 | 5 |

Each phase transferred 256 MiB of payload; tar overhead is excluded from the
throughput numerator. Process CPU and external peak memory include fixture
creation and additional graph checks. Loopback registries, small sample count
and different hardware prevent production capacity or cross-architecture
claims. This is direct registry/core performance, not daemon concurrency or
long-run resource-growth qualification.

The first benchmark attempt expected a nonexistent pull receipt `verified`
field and stopped despite a successful pull. The retained `*-pre-fix.txt` logs
record that harness failure. The corrected runner checks returned artifact,
manifest and platform facts, resolves the target graph, and rehashes its config
and layers through public blob opens. It does not change production receipts.
