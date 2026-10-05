# Native disk-full import boundary — 2026-10-05

The public daemon/CLI ENOSPC fixture passed on native ARM (`apollo-node-01`) and x86 (`tihan-apollo`). It mounts only its newly created private 24 MiB tmpfs, runs artifactd unprivileged, allocates a real operation token before exhausting capacity, and leaves about 4 MiB free before importing an independently hashed 8 MiB file. The CLI reported `No space left on device (os error 28)`. STATUS showed zero blobs, INSPECT rejected the digest, and its final digest leaf was absent.

The fixture SIGKILLs the daemon after that failed request, removes only its own filler, restarts against the same store, and calls RECONCILE. Replaying the original operation returns the identical failure. A fresh operation imports the same bytes successfully; VERIFY reports the expected digest and size. DOCTOR reports clean redb integrity, verified journal, and zero interrupted operations. Its one pending intent is the executing DOCTOR request. The fixture unmounted its own tmpfs on exit.

Exact execution identities:

| Input | SHA-256 |
| --- | --- |
| `scripts/native-disk-full.py` | `f62cb5bf672af12ceeabbbab07b005cc0c73ec7adaff7c8dc810e5a4c46a4232` |
| ARM daemon, `artifactd-cas-repair-20261005.GPrTLa/target/debug/apollo-artifactd` | `8f6cabd652683ad8adac63c8cbc119ec6884a2095ac0df4966aea2df35c61b77` |
| ARM CLI, same target | `74ef16a0e2b0ac656ddac2e13167110a649d660d9f2c668a5bdf18ce7e278f7d` |
| x86 daemon, `artifactd-cas-repair-20261005.Fb6fUB/target/debug/apollo-artifactd` | `557bc328b868077d847d85146e0d9a89c9bec3a30b90ed340869fe1b3dbf95d7` |
| x86 CLI, same target | `8eb35098b921d916305e60b91bbc076dc0840228fefc3e9a9d33334dfde290ea` |
| Imported 8,388,608 bytes | `9dd2dee81ffbd2c1109d074bd53bc33b1ed10cfb9ac43f1c895339b74a411ec9` |

The retained fixture directories are `/home/apollo-admin/artifactd-enospc-_mjyw6sj` and `/home/tihan/artifactd-enospc-41rtj0aq`; daemon stderr is `daemon.log`. Both mounts are no longer active. The x86 result is retained in [disk-full-import-x86_64-20261005.log](disk-full-import-x86_64-20261005.log). Both runs use the previously built corrected CAS recovery binaries, not a newly built whole-workspace release. x86 headroom recovered to about 1.2 GiB before the small fixture ran; no compiler or large churn job was started.

Test-audit authoring gate: the primary observable contract is that an import which exhausts actual filesystem space never exposes partial bytes and remains idempotent through restart. A credible regression is publishing a partial final leaf or replaying a failed operation as success. Existing synthetic size/hash checks and a shared-host churn ENOSPC do not isolate and assert this real filesystem effect/replay boundary. The fixture uses the public CLI, real disk quota, and independent SHA-256 input; it writes no daemon state records and adds no production seam. This is new qualification, not a bug-fix regression with an artificial negative control.

This does not qualify SIGKILL during the full-disk write itself, power loss, ENOSPC during final journal completion, prepared output, or GC. Those remain separate required gates; no release approval is implied.
