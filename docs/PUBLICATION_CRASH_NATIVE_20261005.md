# Native CAS and prepared publication interruption

CAS and prepared-rootfs publication interruption passed on x86_64 and arm64
Linux using the real daemon and Unix API. Prepared-rootfs GC interruption also
passed on both hosts. The final three-mode script SHA-256 is
`ca01eddf883cc1b4953082c53da9c41f8d53844ef565a3b9b45b1ba3033f543e`.
The final script completed each mode on each host: CAS
([x86](publication-crash-final-x86_64-20261005.log),
[ARM](publication-crash-final-arm64-20261005.log)), prepared publication
([x86](prepared-crash-final-x86_64-20261005.log),
[ARM](prepared-crash-final-arm64-20261005.log)), and GC
([x86](gc-crash-x86_64-20261005.log),
[ARM](gc-crash-arm64-20261005.log)). Each result records the exact native
daemon and CLI hashes and the independently checked post-restart state.

`scripts/native-publication-crash.py` starts a newly owned private store under
strace. The maintained Linux tool delays only the return from `linkat`, without
changing its result. The fixture independently hashes the final readonly blob,
observes the two-link publication window, and kills the captured daemon child
before the import request completes. It never writes database records or uses
a production fault-injection flag.

On restart, public RECONCILE and VERIFY succeed. The original operation token
returns the explicit uncertain-outcome failure; a fresh same-digest import
succeeds. The final file has one link, temporary storage is empty, and DOCTOR
reports a clean database and verified live graphs. The one pending journal
intent reported by DOCTOR is its own executing request.

The [raw syscall trace](publication-crash-x86_64-20261005.strace) records a
successful delayed publication followed by SIGKILL. The
[public API result](publication-crash-final-x86_64-20261005.log) records the receipt
and binary provenance.

| Input | Value |
| --- | --- |
| Host | `tihan-apollo`, x86_64 Linux |
| Private fixture | `/home/tihan/artifactd-publish-crash-id2nlmyl` |
| Script SHA-256 | `ca01eddf883cc1b4953082c53da9c41f8d53844ef565a3b9b45b1ba3033f543e` |
| Daemon SHA-256 | `557bc328b868077d847d85146e0d9a89c9bec3a30b90ed340869fe1b3dbf95d7` |
| CLI SHA-256 | `8eb35098b921d916305e60b91bbc076dc0840228fefc3e9a9d33334dfde290ea` |
| Blob | `sha256:7466896d8c733d61ff82701419e8820b5eddc4e2cc3dcd07e8092fb6f61925fc`, 1,048,576 bytes |

These are the focused CAS-repair binaries already qualified for interrupted
streaming import. This CAS-mode result alone does not establish a coherent
final workspace release or storage-device power-loss durability. The
prepared-publication and GC interruption results follow below.

## Test-audit authoring gate

1. The observable contract is recovery of an actual verified CAS publication
   whose caller never received completion, with honest replay and idempotent
   fresh retry.
2. A credible regression leaves the real two-link publication window
   unrecoverable, loses the completed blob, or incorrectly reports success for
   the interrupted operation.
3. Existing persisted-record tests construct that window directly; streamed
   import interruption stops before publication. Neither exercises a real
   successful kernel publication interrupted before userspace completion.
4. The fixture uses the existing public CLI, independent SHA-256, filesystem
   observations and an external syscall tracer. It adds no production seam.

This is new qualification coverage, not a claimed bug regression fix; no
artificial pre-fix negative control is claimed.

## Prepared rootfs publication

The shared fixture now has a `prepared` mode. It creates a standard one-layer
OCI image containing an independently specified 1 MiB file, imports the graph
through the public API, and starts preparation under a delay on the real
`renameat` return. It observes the readonly published directory and correct
file bytes before killing the daemon. The
[prepared syscall trace](prepared-crash-x86_64-20261005.strace) proves the
successful rename followed by SIGKILL; the
[public result](prepared-crash-final-x86_64-20261005.log) records recovery and receipt.

Restart reconciles the real prepared intent. Original-token replay reports
uncertainty. Fresh PREPARE returns the same published identity, its manifest
and platform, a verified prepared digest, and the expected size. A covering
lease then permits OPEN_PREPARED to transfer a readonly directory FD; reading
the expected file relative to that FD returns the independently specified
bytes. The fixture releases the lease, verifies staging is empty, and runs
DOCTOR. It executes no image commands.

This guards a different recovery owner from CAS: the prepared tree must be
verified and promoted from its durable intent before descriptor publication.
Persisted-record preparation tests cannot reach the real successful rename
interrupted before completion. The fixture extends the same public lifecycle
driver and requires no production test seam.

| Input | Prepared-mode value |
| --- | --- |
| Script SHA-256 | `ca01eddf883cc1b4953082c53da9c41f8d53844ef565a3b9b45b1ba3033f543e` |
| Private fixture | `/home/tihan/artifactd-publish-crash-e4cj21r2` |
| Manifest | `sha256:3c321eeb8fd8e6c1b52ce82c0bbb44bf6039ecdd2734a7a069900408a509039f` |
| Prepared identity | `e9c03afbd1cce252951b800789a0a15cc6083a792aa543904220ee4a1c71a367` |
| Prepared digest | `sha256:1d13e17faf2051cfd9b2d47dd676d24942032d87a52880d0e38c006f97792c4d` |

The daemon and CLI hashes are the same focused CAS-repair binaries listed
above. This adds prepared publication SIGKILL evidence, without claiming
power-loss durability or the complete preparation/GC fault matrix.

## Prepared rootfs GC interruption

The fixture first prepares the same one-layer OCI image through the public
API, then starts the daemon under a delay on the first real `unlinkat` effect
after listener setup. On both hosts, [x86 syscall trace](gc-crash-x86_64-20261005.strace)
and [ARM syscall trace](gc-crash-arm64-20261005.strace) show successful deletion
of `verified-file` followed by SIGKILL. The fixture observes the partial tree
while the original GC request is incomplete.

On restart, RECONCILE removes the owned partial tree. Original-token replay
reports an uncertain outcome; fresh bounded GC completes with zero blobs and
no prepared output. DOCTOR reports a clean database and verified live graphs.
See the [x86 public result](gc-crash-x86_64-20261005.log) and
[ARM public result](gc-crash-arm64-20261005.log). The x86 and ARM daemon/CLI
hashes are the focused repair binaries recorded in the corresponding result.

This guards the crash recovery of a real partially deleted rootfs tree.
Synthetic persisted-record GC tests cannot prove that the effect actually
removed a file before the daemon died. The test uses a private image, actual
GC request and external syscall tracer; it adds no production test seam.

The first GC fixture attempt on each host delayed stale socket cleanup during
daemon startup, before GC began. Those attempts did not qualify GC. The driver
now removes only its stopped daemon's private stale socket before tracing,
and limits delay to the first GC deletion syscall. The passing traces name
`verified-file`, proving the intended effect was reached.

An attempt to run an older prebuilt x86 `gc_crash` test executable failed in
its daemon readiness probe before exercising GC. Its binary and daemon were
not established as a coherent current build, so that result is excluded from
GC qualification. The public fixture above uses the paired focused CAS-repair
daemon and CLI and reaches the real deletion effect. A final coherent source
build and full native suite are still required for release.
