# Public API churn campaign — 2026-10-05

Current 132-file core/example snapshots match local authoritative sources on both native hosts, recorded in `api-churn-current-{x86_64,arm64}-snapshot-20261005.json`. Strict workspace/all-target Clippy and release builds (both binaries plus `api_churn`) passed. The 10 blob + 5 layered OCI smoke passed on each host with empty final owned content directories/tables and bounded operation journals. Logs: `api-churn-preflight-{x86_64,arm64}-20261005.log`.

The first smoke launch wrapper had a Python quoting error after successful compilation. No daemon/workload ran in that failed wrapper; the corrected wrapper ran the unchanged compiled example successfully. Earlier outdated source preflight and the x86 Vec-argument lint are not treated as passing evidence.

Long campaigns are active, not yet qualified:

- ARM `apollo-node-01`: tool session 41804.
- x86 `tihan-apollo`: tool session 56807.
- Each runs 100,000 blob lifecycles + 20,000 nonempty layered OCI import/pin/prepare/open/release/unpin/GC lifecycles through the actual Unix daemon API.
- Remote log on each: `~/artifactd-qualification-20261004/public-api-churn-20261005.log`.
- Each creates a separate private `~/artifactd-api-churn-20261005-*` directory retained for inspection; exact path is first JSON log record.
- Both handles were polled and confirmed running after source snapshots were captured. Do not mutate these checkouts/targets until terminal.

Campaign success requires terminal exit zero, `PUBLIC_API_CHURN=PASS`, final empty content/reference/GC inventories, bounded operation journal and retained resource/latency checkpoints. These campaigns do not qualify the full interrupted-effect crash matrix, registry throughput, or global red-team release gate.

## x86 original failure and retry

Original x86 handle 56807 terminated with exit one after passing the 30,000-blob checkpoint. Import returned `ENOSPC`; this is a failed campaign, not a pass. The private retained store is `/home/tihan/artifactd-api-churn-20261005-1i_c2t_p`. At subsequent inspection the persistent filesystem had 28 GiB free, while separate `/tmp` tmpfs was 93% full; the exact exhausted mount at the failure instant is not established.

A restart using the unchanged qualified daemon succeeded. Public STATUS before/after RECONCILE reported zero blobs/bytes; DOCTOR reported clean redb integrity and one retained pending journal intent. Recovery log: remote `public-api-churn-disk-recovery-20261005.log`. This real disk-pressure observation does not substitute for controlled disk-full fault qualification.

Fresh x86 retry handle 9647 uses a separate private campaign directory and sets TMPDIR to its owned persistent-filesystem `inputs` directory. Log: `public-api-churn-retry-20261005.log`. ARM original handle 41804 remains active. No terminal retry result is yet qualified.

The retry directory is `/home/tihan/artifactd-api-churn-retry-20261005-ve5g_3y3`. Subsequent live observations reached 10,000 blob cycles on x86 (RSS 15,848 KiB, 14 FDs, one thread), and 60,000 on ARM (RSS 57,376 KiB, 13 FDs, one thread). State bytes were 3,538,944 on both. Increasing RSS remains an unresolved bounded-growth qualification concern; these checkpoints do not establish a memory plateau. Original x86 failure and restart logs are now retained locally as `api-churn-x86_64-failed-20261005.log` and `api-churn-x86_64-disk-recovery-20261005.log`.

Read-only inspection of a private copied x86 failed-store database found zero import intents and zero journal intent records, plus exactly one terminal failed ENOSPC import. DOCTOR counted its own executing journal intent before completion. See `api-churn-x86_64-disk-recovery-inspection-20261005.md` for copied-state hash, phase counts and the source explanation. This resolves the earlier apparent pending-intent concern; it does not turn the failed churn campaign into a pass.
