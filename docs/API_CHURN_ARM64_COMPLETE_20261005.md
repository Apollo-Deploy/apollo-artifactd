# Native ARM public API churn result — 2026-10-05

The original owned ARM campaign completed with `PUBLIC_API_CHURN=PASS`: 100,000 distinct blob operations and 20,000 layered OCI import/pin/lease/prepare/unpin/GC lifecycles, through the actual daemon protocol. The retained log is `api-churn-arm64-complete-20261005.log`; its exact executable/source provenance is the earlier `api-churn-current-arm64-snapshot-20261005.json`. This campaign predates the subsequent interrupted-stage recovery and Rust FFI edits, and does not qualify those later revisions.

The final persisted content, import, reference, prepared, registry and GC tables were empty; the operation journal retained 4,066 rows within its 4,096-row bound. Blob/temp/prepared directories were empty after restart and final reconciliation. FDs stayed at 13–14 and the daemon reported one thread. The state file plateaued at 3,538,944 bytes.

Blob-cycle latency p50/p95/p99 was 18.223/28.868/387.062 ms. OCI-cycle latency was 36.949/121.167/785.531 ms. Elapsed time was approximately 3,987 seconds. These timings include concurrent qualification activity on a shared host; they are not isolated production capacity benchmarks.

Memory qualification remains open: blob-phase RSS grew from 5,336 KiB to 83,532 KiB before the deliberate restart. Restart returned RSS near baseline; this does not prove bounded long-running memory. The isolated allocator investigation is recorded separately in `RSS_INVESTIGATION_20261005.md`.

The x86 retry reached the 90,000-blob checkpoint and terminated with ENOSPC before a completion marker. `api-churn-x86_64-retry-failed-20261005.log` is failure evidence, not a passing churn result. No required release gate is waived by this ARM result.
