# x86 disk-pressure recovery inspection

This is a read-only inspection of the isolated failed campaign store
`/home/tihan/artifactd-api-churn-20261005-1i_c2t_p` on `tihan-apollo`. The
active qualification stores and processes were not opened or modified.

The retained log is
`docs/api-churn-x86_64-disk-recovery-20261005.log`. It records an ENOSPC
`IMPORT_BLOB`, followed by `RECONCILE` and `DOCTOR`. The DOCTOR response showed
one pending intent at that point in the request sequence.

For inspection, only `store/state.redb` was copied to a private temporary
directory and opened with the public `State` adapter. The copied file was
3,538,944 bytes and had SHA-256:

```text
446764d4ee3bfc0282b1566b52552fb887f888d3c3fb979685d8252c3a2298f3
```

The sanitized state scan reported exactly:

```text
operations=4085
allocated=583
complete=3501
failed=1
intent=0
imports=0
```

The sole failed record was sequence `264755`, an `IMPORT_BLOB` whose terminal
result was `No space left on device (os error 28)`. No request payloads or
digests from the other records were retained in this note. The store contained
only `owner.lock` and `state.redb`; there were no blob or temp files.

The source interpretation is anchored to these hashes from the inspected
checkout:

```text
src/api/journal_recovery.rs 06747f60df03b0bd8f4dae474cbfed03a5da2cfbc22460b6ccae0ee25b0532f5
src/api/journal.rs           c4cd7f70467ef14ade9e83b53c2d33c92ab4cc96caca687e371081b2559404f4
src/cas/recovery.rs          76c0e71b4b93972c942ab82aeb8179d56bae62886b3daaa61052c17199228292
```

`journal_recovery::audit` counts only `phase == "intent"` as interrupted.
The 583 allocated records have empty requests and results, so they are unused
reservations. The ENOSPC import is terminally failed, not an unresolved effect
intent. The one-intent DOCTOR value is therefore an intermediate snapshot from
the recovery request sequence; the persisted post-recovery copy contains no
intent record. This observation does not establish a recovery defect.

## RSS checkpoint comparison

The active x86 retry log was inspected without attaching to or changing its
process. Its daemon PID was `2373758` and the runner reported:

```text
iteration 0      VmRSS  6,692 kB   state.redb  94,208 bytes
iteration 10,000 VmRSS 15,848 kB   state.redb 3,538,944 bytes
iteration 20,000 VmRSS 25,288 kB   state.redb 3,538,944 bytes
```

The later read-only `/proc` sample at iteration 20,000 reported VmRSS/HWM
26,712 kB, one thread, 20,948 kB private anonymous RSS, and a 20,376 kB heap
mapping. `AnonHugePages` was 18,432 kB. The state file had stopped growing at
3.5 MiB while the private heap continued growing, and transparent huge pages
accounted for most of the x86 heap RSS. This is evidence of allocator/page
retention or live churn working set, not evidence that redb state or a journal
vector is growing without bound.

The checkpoints are too sparse to distinguish allocator retention from a true
object leak. A controlled isolated run must sample `smaps_rollup`, `[heap]`,
`AnonHugePages`, state size, table counts, FDs, and threads at every checkpoint;
heaptrack and valgrind were unavailable on the inspected hosts. No bounded-RSS
or leak-free qualification is claimed from these observations.

## Why DOCTOR observes one intent

Source inspection explains the intermediate counter without an unresolved import: `Action::Doctor` is a mutation (`crates/artifactd-protocol/src/lib.rs`, `Action::is_mutation`). `src/api/dispatch.rs` persists its own journal intent before invoking `store.doctor()` and `journal_recovery::audit(store, false)`, and commits the DOCTOR completion afterward. The audit therefore includes the currently executing DOCTOR request. The copied post-request state contains zero intents. No counter exclusion or recovery behavior was changed for this observation.
