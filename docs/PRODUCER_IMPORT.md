# Producer-calculated import qualification

`IMPORT_BLOB` accepts an omitted/null expected digest for a trusted local producer. The size remains required. Existing requests containing a digest retain expected-digest verification. Both paths stream through a 64 KiB buffer; unknown identity is never published under a guessed name.

Intent order: persist unresolved import and quota reservation → create private temp → stream/hash and verify size → fsync/read-only temp → persist resolved SHA-256 → atomically publish/deduplicate → verify final bytes → commit admission and intent completion. A restart discards unresolved private staging files. Resolved publication uses the existing inode-pair recovery path.

## Test-audit authoring gate

The test-audit skill was applied to the new tests. The direct public Store test independently computes expected SHA-256, reads bytes after restart and checks deduplication and size rejection. Credible failures include returning the declared identity without hashing, admitting an incorrectly sized stream or duplicating objects. Expected-digest-only coverage could not call the missing producer entry point.

The daemon integration extension checks the distinct transport contract: a request with a null digest and a real regular input FD returns generic digest/size facts. It reuses the existing daemon fixture rather than adding a copied API test.

The actual SIGKILL test checks the distinct unresolved-intent recovery path. It uses the production Read input interface, waits for real partial filesystem effects, kills the child process, restarts the store and proves no admitted blob or staging file remains. The pause and mode selection live only in the test executable, not in production exports/flags. It shares setup with the prior expected-digest crash case.

## Evidence

Focused local correctness and SIGKILL tests pass. Native arm64/x86_64 focused logs are `producer-import-{architecture}-tests.txt`; each includes public Store correctness, real SIGKILL recovery and the daemon FD/API test. Full-suite evidence is recorded separately when completed. The baseline 100k/20k churn and throughput snapshots predate this change; those fixtures exercise expected-digest imports and do not qualify the new producer path.

This closes the missing producer-import feature, not registry transfers, corrupt CAS repair, full recovery qualification or release approval.
