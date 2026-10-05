# CAS repair native qualification — 2026-10-05

The isolated native x86_64 and aarch64 checkouts recorded in `cas-repair-native-{x86_64,arm64}-snapshot-20261005.json` each passed `cargo test --locked --test cas`: 14 passed, zero failed. Logs are retained as `cas-repair-native-{x86_64,arm64}-20261005.log`.

The frozen import source was `1a61cc22acefeed4a60e818ed418f6b91f3c32dc1362d219a45ffe2f5533a728`. It covers owned corrupt replacement, input refusal, pin retention, known effect-window recovery, unrecorded collision preservation, and logical peak staging quota. It precedes the subsequent actual oversized-corrupt-file accounting and missing-final case additions, which require a separate native rerun. The active public churn checkouts and targets were not modified by these runs. Compilation on these shared hosts can affect observed churn timings; the churn metrics are qualification-host observations, not isolated capacity benchmarks.

Independent source review is recorded in `RED_TEAM_CAS_REPAIR.md`; no global release approval or full power-loss/crash matrix is implied.

## Final repair revision

Both isolated native checkouts subsequently passed `cargo test --locked --test cas_repair` (2 tests) and `cargo test --locked --test cas` (14 tests), with zero failures or ignored tests. Final logs are `cas-repair-native-{x86_64,arm64}-final-20261005.log`; final source hashes are recorded separately in `cas-repair-native-{x86_64,arm64}-final-snapshot-20261005.json`. The import hash is `7b5dd9a798dd343cce1e249a080cbfb4df1e2208cfe6c9c6a8870717eed9c5e6` on both hosts and matches the local authoritative source.

These runs additionally prove that actual oversized corrupt-file bytes count against peak staging quota, insufficient quota preserves the existing file without publishing staging bytes, adequate quota permits verified repair, and a recorded but missing final blob can be reimported. The oversized-quota negative control removed only the extra physical-byte accounting and failed the intended refusal assertion locally. These are focused CAS qualification results, not the full daemon churn, registry, power-loss, or global release gate.

## Interrupted-stage recovery follow-up

Registry SIGKILL testing found a distinct writable-partial recovery regression after the revision above. The corrected recovery hashes and actual native reruns are recorded in `REGISTRY_CURRENT_NATIVE_20261005.md` and `cas-interrupted-native-{x86_64,arm64}-snapshot-20261005.json`. Both native reruns additionally pass the same 14 CAS and 2 repair tests plus recovery cases. The earlier frozen hashes remain historical evidence; use the newer snapshots for the repaired interrupted-stage implementation.

A later test-helper-only edit initializes `Limits.max_store` directly with a struct update instead of assigning after `Default`. Local strict core Clippy (`--lib --tests -D warnings`) passed. Native logs above cover the preceding semantically equivalent helper source; their snapshots have not been rewritten to claim execution of this cosmetic edit.
