# Current native registry qualification — 2026-10-05

The isolated CAS qualification checkouts on `tihan-apollo` and `apollo-node-01` each passed the authenticated HTTPS registry store test and the daemon API credential-FD/journal/prepared-facts test. The source capture is the initial `cas-repair-native-{arch}-snapshot-20261005.json` with subsequent CAS source changes recorded in `cas-repair-native-{arch}-final-snapshot-20261005.json`; these tests precede later FFI workspace changes and any interrupted-staging recovery fix. No active churn checkout or target was changed.

Evidence retained:

- `registry-store-current-{x86_64,arm64}-20261005.log`: one passed test per architecture, covering repeated pushes, digest-pinned pulls/local reuse, wrong reference/digest refusal, authentication failure and credential-error redaction, verified prepared payload.
- `registry-api-crash-current-{x86_64,arm64}-20261005.log`: the daemon API test passes on each architecture, followed by a failed interrupted-pull test.
- `registry-credential-path-error-{x86_64,arm64}-20261005.log`: first launch used a nonexistent credential-provider filename; corrected launch results are recorded separately. No transfer succeeded in the initial launch.

The x86 interrupted-pull test failed importing its source layer with ENOSPC before reaching the kill window. Subsequent inspection showed separate `/tmp` tmpfs at 96% use and the persistent filesystem with 22 GiB available. A private persistent TMPDIR is required for rerun; this failure is not a crash-recovery pass.

ARM reached the actual partial-layer SIGKILL window, then failed daemon restart with `published content is writable`. Inspection found that initial import intent records already contain the expected digest before staging finishes, while recovery attempts final/read-only verification of the interrupted writable partial stage. This is a confirmed restart qualification failure requiring an owner repair and rerun; the earlier focused CAS repair results do not cover it.

These results do not establish full registry fault/auth/redirect/security/performance qualification or global release approval.

## Interrupted-stage owner repair and ARM rerun

Recovery now discards only private, owned, single-linked incomplete stages, or readonly exact-size stages whose successful hash proves a digest mismatch. Ownership/open/hash failures remain fail-closed; complete publication still requires readonly verified content and exact inode-pair evidence where applicable. The unused staged-verification helper was removed from import.rs. A persisted readonly wrong-digest crash-window regression passes locally.

The frozen repaired source is captured in `cas-interrupted-native-arm64-snapshot-20261005.json`: recovery `cdaa050be9dce368525f92b33c8ddc91185fcfcb9c82a8c1e91ea67bbb2ff648`, import `91b6362ac4e00c4d2073e17cc3965ab9cc9372d6df3391f5d7a353540934b7eb`, and recovery test `4ea8e70c1ed630e51351f6b29226ad041b9bdc6d7466630b52d1c364edc29e53`. Native ARM rerun passed the actual partial registry-pull SIGKILL/restart/uncertain replay/fresh pull test (117.34 seconds), followed by CAS 14, repair 2, and recovery 8 passing test entries. The recovery target includes a subprocess helper entry, so its count is not eight independent recovery invariants. Log: `cas-interrupted-native-arm64-20261005.log`. All temporary fixture inputs used a private persistent directory.

The x86 equivalent rerun subsequently completed with exit zero: actual interrupted registry pull, CAS 14, repair 2 and recovery 8 passing test entries, retained in `cas-interrupted-native-x86_64-20261005.log`. Its source hashes match the repaired ARM revision and are recorded in `cas-interrupted-native-x86_64-snapshot-20261005.json`. Scoped independent review is in `RED_TEAM_INTERRUPTED_IMPORT_20261005.md`; its local test scope is distinct from these native reruns and it does not approve the whole service release.
