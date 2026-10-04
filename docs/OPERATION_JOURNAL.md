# Bounded operation replay

Protocol version 2 requires daemon-issued tokens for mutations. Tokens contain
a random per-store epoch and a monotonically increasing sequence. Allocation,
intent, completion, and retirement use redb transactions with immediate
durability. The CLI allocates tokens automatically; callers retain the returned
token to retry the same request.

The journal keeps at most 4096 reservations/results. When full, allocation
audits the bounded retained inventory and retires up to 64 oldest terminal
records or unused reservations, atomically advancing the retirement floor.
Tokens below the floor fail with `operation expired`; guessed future tokens,
foreign epochs, and conflicting request reuse fail before effects. No
unbounded tombstones are needed. Expired or uncertain operations require
inspection before a caller decides to submit another mutation.

Both successful and failed results replay while retained. A persisted intent
with no completion has an uncertain outcome: an effect may have completed
before the completion commit failed. Startup converts such intents to explicit
terminal failures without executing them again. A live retry of an unresolved
intent does the same; allocation pressure resolves remaining unknown intents
before retirement. This relies on the daemon's serial request execution. A
future concurrent dispatcher must introduce operation ownership before using
this recovery policy.

Startup and `DOCTOR` scan at most 4096 records, in pages of 64, checking
contiguous canonical token keys, sequence correspondence, mutation request
shape, phase/result consistency, and exact registry companion parity. This is
semantic journal verification, not proof of redb structural integrity.

## Evidence

- [Actual pre-fix ARM daemon](journal-capacity-pre-fix-arm64.txt): commit
  `da3ffb4` accepts operations 1 through 4096, then permanently rejects 4097.
- [ARM API checks](journal-native-arm64-api.txt): real Unix descriptor API and
  protocol token tests pass.
- [ARM journal and registry checks](journal-native-arm64-tests.txt) and
  [x86 journal and registry checks](journal-native-x86_64-tests.txt): more than
  4096 successful mutations, expired retry rejection, failed replay/conflict,
  persisted interrupted-intent recovery, corrupt registry companion refusal,
  and authenticated HTTPS push/pull pass.

The persisted-intent fixture deliberately reproduces a missing completion at
the database boundary. Separate [ARM SIGKILL checks](journal-crash-arm64.txt)
and [x86 SIGKILL checks](journal-final-x86_64-checks.txt)
interrupt real daemon blob imports and authenticated registry pulls during
observable partial CAS writes, then verify uncertain token replay and cleanup.
Both fixtures stream 256 MiB. These cover interruption during transfer, not
every possible effect/completion publication window.

The final native workspace suites and warning-free clippy checks passed on
both architectures. The [ARM source snapshot](journal-qualified-arm64-snapshot.json)
and [x86 source snapshot](journal-qualified-x86_64-snapshot.json) each match
70 current source, test, dependency, and deployment files. Ignored campaigns
were invoked explicitly and are recorded separately from the normal suites.

[ARM](journal-churn-arm64.txt) and [x86](journal-churn-x86_64.txt) each completed
100,000 public allocation + unpin cycles and a mutation after restart. These
are debug builds and journal availability measurements, not replacement proof
for blob/OCI lifecycle churn or release throughput. Latencies include allocation
and mutation, with durability and retirement verification enabled.

| Measurement | ARM | x86 |
|---|---:|---:|
| Elapsed seconds | 419.083 | 819.731 |
| p50 / p95 / p99 microseconds | 3399 / 4029 / 47705 | 5527 / 7615 / 160618 |
| Baseline / peak RSS KiB | 9632 / 17544 | 12680 / 21520 |
| Threads | 1 | 1 |
| Baseline / maximum FDs | 13 / 13 | 13 / 14 |
| Daemon CPU ticks (100 Hz) | 27079 | 79641 |
| Final retained operations | 4064 | 4064 |
| Database bytes | 2707456 | 2707456 |

The p99 includes the cost of a full bounded semantic audit at retirement
pressure. Verification and immediate durability remain enabled.

Legacy nonempty journal migration, database corruption, full artifact lifecycle
churn, release performance, and release red-team approval remain mandatory work.

The x86 crash log retains two initial failures: the source fixture encountered
`ENOSPC` on the separate `/tmp` tmpfs, and a subsequent large fresh pull exceeded
the original 30-second client response deadline. Qualification fixtures now
use an owned temporary directory on the ordinary disk for that host. This is
not a controlled disk-full recovery qualification. The production client waits
up to 600 seconds for a completed operation response; daemon request-packet and
response-send deadlines remain 30 seconds. A client timeout does not authorize
a new operation: retain the allocated token and retry it, or inspect an expired
or uncertain outcome before deciding on a new token.
Nonempty version-1 journals are preserved and rejected; no old protocol
fallback is retained.
