# Current-workspace native public API churn — 2026-10-05

Two fresh, private campaigns used the coherent 172-input release
build described in [CURRENT_WORKSPACE_NATIVE_20261005.md](CURRENT_WORKSPACE_NATIVE_20261005.md).
The local, x86_64 and ARM64 source aggregate is
`aed9b479920deeda2e82e63eee2291c4378b2de7a08f858ed3a7b5e3a7bd83b1`.
Each runner targets 100,000 blob operations and 20,000 one-layer OCI
import/pin/lease/prepare/open/unpin/GC lifecycles through the real Unix API.
The source includes the subsequent CAS-recovery and FFI changes absent from
the earlier completed ARM campaign.

| Host | Private run directory | Release daemon SHA-256 | Runner SHA-256 |
| --- | --- | --- | --- |
| `ssh tihan-apollo` (x86_64) | `/home/tihan/artifactd-api-current-20261005-56l_0rhk` | `eff54323304eaf33b58541b8cd96627e1184a0f3824f525f8942e1060a96c5b1` | `4d1efb18852fc4a8e48d65d6ec56d8f30e437bdeddbb4870f3d44738b30ba772` |
| `ssh apollo-node-01-internet` (aarch64) | `/home/apollo-admin/artifactd-api-current-20261005-b3y_d42z` | `02cacdbc4a936c9f0238feedcba522df72ab2b7746210ec32166d7bbe4a282ab` | `0c6ea3b499ca0745baf102ea4fd35446f09f79be54c80dd0826e2fafdaa7cdf5` |

Each directory retains `manifest.json`, `run.log`, `launcher.log`, launcher and
child PIDs, and eventually an `exit` receipt. An independent launcher monitors
available space and terminates its own child group below 1.5 GiB, writing
`disk_guard`; that guard is a host-protection limit, not a passing result.
The run directories and live process groups must remain undisturbed until
terminal observation.

At the initial capture both launchers and runners were live with no `exit`
or `disk_guard` receipt. At the 10,000-blob checkpoint, the x86 daemon reported 16,420 KiB
RSS, 14 FDs, one thread and a 3,538,944-byte state file; the ARM daemon
reported 15,236 KiB RSS, 13 FDs, one thread and the same state size. Available
filesystem space was 4.64 GiB and 21.49 GiB, respectively. This is an interim
observation, not churn completion or a demonstrated RSS bound.

## x86 terminal result

The x86 runner exited zero after all 100,000 blob operations and 20,000 OCI
lifecycles. It retained an empty final content/reference/prepared/GC inventory,
4,066 operation records (limit 4,096), zero blob/temp/prepared filesystem
entries, and five store entries. State plateaued at 3,538,944 bytes; checkpoints
showed 13–15 FDs and one daemon thread. The workload took 3,416.956 seconds.
Blob-cycle latency p50/p95/p99 was 21,932/37,441/43,904 microseconds; OCI-cycle
latency was 47,347/67,681/77,687 microseconds. Other owned diagnostics ran on
the hosts, so these measurements are workload observations, not controlled
throughput comparisons.

The memory bound **failed**: pre-restart RSS grew to 85,828 KiB despite the
flat state file. Restart reduced RSS to 8,652 KiB. Successful fixture completion
and cleanup do not pass the bounded-memory gate. See the
[allocation-owner investigation](RSS_CURRENT_RELEASE_20261005.md).

[Terminal receipt and resource summary](api-churn-current-x86_64-terminal-20261005.json)
and the [complete log](api-churn-current-x86_64-complete-20261005.log) retain
this result. Log SHA-256:
`869c7bb5e2a6263c9ff718cf83fbdae1724afcc95fcc3c81cdb708d6bf39df1f`.

## ARM terminal result

ARM also exited zero after 100,000 blob operations and 20,000 OCI lifecycles.
Its final inventory matched x86: 4,066 operation records, all other lifecycle
tables empty, zero blob/temp/prepared entries and five store entries. State
plateaued at 3,538,944 bytes; checkpoints showed 13 FDs and one daemon thread.
Elapsed time was 4,447.366 seconds. Blob-cycle latency p50/p95/p99 was
20,102/41,857/446,148 microseconds; OCI-cycle latency was
37,793/83,661/767,853 microseconds. Concurrent profiling and builds affected
host load; these are observed campaign latencies.

ARM's pre-restart RSS reached 85,884 KiB; final post-restart RSS was 7,668 KiB.
Its memory bound also **failed**. The [ARM terminal receipt](api-churn-current-arm64-terminal-20261005.json)
and [complete log](api-churn-current-arm64-complete-20261005.log) retain the
result; log SHA-256:
`a2823d791da676e03b197f027e9306dbd7f782e4cf7fc6d507f72d80eeb6c5ef`.

Both original campaigns predate the vendored redb queue repair. Their
resource growth does not qualify the repaired source. A fresh repaired ARM
campaign subsequently completed those counts in a separate private store. The
[manifest](redb-lru-api-churn-arm64-manifest-20261005.json) and
[terminal summary](redb-lru-api-churn-arm64-summary-20261005.json) record the
source and executable hashes. It exited zero after 100,000 blob imports and
20,000 layered OCI lifecycles. Pre-restart sampled RSS/HWM peaked at 9,404 KiB;
state was 3,538,944 bytes, with 13 FDs and one thread. All content and lifecycle
inventories were empty at completion, with 4,066 operation rows within the
4,096 bound. Blob latency p50/p95/p99 was 18,735/47,568/296,066 microseconds;
OCI latency was 36,980/108,811/740,926 microseconds. This passes the memory
gate for that source revision. Subsequent current-source reruns on both
architectures are recorded in [REDB_LRU_API_CHURN_NATIVE_20261005.md](REDB_LRU_API_CHURN_NATIVE_20261005.md).
Broader concurrency/load qualification remains open.

Acceptance requires terminal exit zero, the requested completed counts, final
empty owned content/reference/prepared/GC inventories, bounded operation
state, and the complete resource and latency record. This runner does not
emit `PUBLIC_API_CHURN=PASS`; requiring that nonexistent marker was a
reporting error. Bounded memory requires a stable resource trajectory in
addition to those functional checks. These campaigns alone do not prove the
interrupted-effect crash matrix, registry load, or release approval.
