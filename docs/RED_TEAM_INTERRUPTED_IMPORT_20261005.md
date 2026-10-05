# Red-team review: interrupted CAS import recovery

Date: 2026-10-05  
Mode: implementation review, scoped to interrupted-import recovery and repair  
Verdict: **Non-blocking issues; scoped repair survives the executed attacks.**  
This is not a global release approval. The live core checkouts and `~/artifactd-qualification-20261004` were not modified.

## Review charter

The fixed safety properties are: a crash must not publish partial bytes; a recovery cleanup must only remove a private, daemon-owned regular file; a hardlink or symlink supplied at the stage path must fail closed; a digest or size mismatch must not replace an existing final object; a recorded corrupt final may be repaired only through verified replacement; and repair admission must account for the physical peak while old bytes and staged bytes coexist. The review does not authorize production edits, same-UID compromise hardening, or native qualification reruns.

The relevant trust boundary is the private mode-0700 store owned by the artifactd service UID. Public callers do not receive directory capabilities or pathname mutation APIs. A process already running as that service UID is outside the supported adversary boundary; it can directly alter both the database and store files.

## Reviewed revisions

| Artifact | Content SHA-256 |
| --- | --- |
| `src/cas/import.rs` | `91b6362ac4e00c4d2073e17cc3965ab9cc9372d6df3391f5d7a353540934b7eb` |
| `src/cas/recovery.rs` | `cdaa050be9dce368525f92b33c8ddc91185fcfcb9c82a8c1e91ea67bbb2ff648` |
| `src/filesystem.rs` | `fedaaa3b91eadd8bb69535343058c1babe461ad599308fdaf2a4dc6f70c991c1` |
| `tests/recovery.rs` | `4ea8e70c1ed630e51351f6b29226ad041b9bdc6d7466630b52d1c364edc29e53` |

## Primary findings and adjudication

### CAS-INT-001 — same-UID pathname replacement race

Priority: low under the stated deployment boundary; medium if a same-UID attacker is in scope. Confidence: high.

`recovery.rs:70-86` checks whether the final digest path exists and then uses `Dir::rename` to publish a verified private stage. A same-UID process with write access to the private directories could create or replace the destination after the check and before `rename`; the rename can replace that path. `filesystem::owned_remove` has the analogous check-then-unlink window (`filesystem.rs:111-120`). Descriptor-relative `NOFOLLOW`, UID, regular-file, and link-count checks prove the object opened at the check, but do not make the later pathname operation compare-and-swap.

Disposition: **REJECTED as a release-blocking public vulnerability; retained as a hardening note.** The service uses a dedicated UID, private 0700 directories, and an exclusive owner lock. Public callers cannot reach this mutation boundary. A compromised service-UID process already has equivalent ability to rewrite the DB and final files. If co-UID adversaries become an explicit requirement, use a no-replace publication primitive and a race test before changing the verdict.

### CAS-INT-002 — foreign symlink and hardlink deletion

Priority: high if it succeeds; result: no confirmed issue. Confidence: high.

The attack model uses a symlink at the recorded `.part` path, a hardlink with `nlink > 1`, a foreign-UID file, and a writable or wrong-sized stage. `symlink_metadata` sees the link without following it; `owned_remove` opens with `NOFOLLOW`, then `private_fd` requires the current UID, a regular file, and `nlink == 1`. The publication branch additionally requires an exact device/inode match between the temp and final entries and a readonly, private inode. Source analysis shows these checks cause symlinks, foreign files, and unrelated hardlinks to error and preserve the import intent. The Darwin fixture executed here covered the symlink/external-target case only; it did not execute different-UID or hardlink cases.

Disposition: **REJECTED.** The owner boundary is enforced in `filesystem.rs:111-120` and recovery leaves unproven records durable rather than authorizing cleanup.

### CAS-INT-003 — writable or wrong-sized interrupted stage

Priority: high; confidence: high.

The original failure mode is an intent with `digest: Some(...)` persisted before a partial writable stage. Recovery now sends writable or wrong-sized stages through `owned_remove`, removes the intent only after that ownership check succeeds, and otherwise fails closed. A private readonly stage is hashed before a digest mismatch can authorize discard (`recovery.rs:45-68`).

Disposition: **REJECTED.** The focused regression `readonly_wrong_digest_stage_is_discarded_after_owner_verified_hash` passes, and the SIGKILL tests prove partial content is not exposed as a blob.

### CAS-INT-004 — digest and size confusion during repair

Priority: high; confidence: high.

The attack supplies a correct declared digest with wrong bytes, wrong size, a missing recorded final, or a corrupt final larger than the recorded DB size. `import.rs:43-62` requires the recorded logical size to match the request, measures the actual private final inode, and includes excess physical bytes in peak quota. `publish_blob` and recovery require a matching recorded blob before replacing an existing corrupt final; an unrecorded collision is preserved.

Disposition: **REJECTED.** `cargo test --test cas_repair` passed 2/2, including missing-final repair and oversized-corrupt-final quota refusal/preservation. The CAS suite passed 14/14, including wrong-input refusal, unrecorded collision preservation, and corrupt recorded repair.

### CAS-INT-005 — error cleanup and publication windows

Priority: high; confidence: high.

The attack crashes or fails after staging, after hard-link publication, after replacement rename, or before the DB transaction. The intent is durable before publication. The normal path retains the intent if cleanup cannot prove ownership. Recovery recognizes only the exact two-link inode publication window, verifies the final bytes before writing the blob record, and can complete a valid readonly stage after the intent update.

Disposition: **REJECTED for the reviewed cases.** `cargo test --test recovery` passed 8/8, including both SIGKILL cases, publication-window reconciliation, and idempotent reopen behavior. The CAS suite also passed both publish-effect-window cases.

### CAS-INT-006 — quota and bounded recovery regressions

Priority: medium; confidence: high.

Repair admission allows replacement when object count is saturated, but still enforces pending-import and temporary-byte limits. Store usage includes blobs, temporary imports, prepared bytes, incoming stage bytes, and physical corrupt-final excess. Recovery remains bounded by `max` and the prepared-GC cursor tests exercise late records.

Disposition: **REJECTED for the exercised regressions.** `cargo test --test cas_repair` passed the peak-quota negative and positive cases; `cargo test --test recovery` passed the bounded cursor and prepared-GC cases. Integer additions are checked and overflow returns an error.

### Owner-boundary negative control

To verify that the wrong-digest regression test reaches the owner-boundary repair, I copied the checkout to `/tmp/artifactd-negative-src.8LIoDN`, used a separate `/tmp/artifactd-negative-target.m56q0G`, removed only the new readonly-stage digest-mismatch discard block from the copied `src/cas/recovery.rs`, and ran:

```text
CARGO_TARGET_DIR=/tmp/artifactd-negative-target.m56q0G \
  cargo test --test recovery readonly_wrong_digest_stage_is_discarded_after_owner_verified_hash \
  -- --exact --nocapture
```

The test failed as intended with `blob integrity mismatch` at `tests/recovery.rs:93`, when the retained wrong stage remained and the reopen assertion attempted to read the expected blob. This is an isolated source/target negative control; the shared checkout and its production sources were not mutated. The control supports that the passing regression test exercises the new discard behavior rather than merely setup or compilation.

## Counterexample execution log

| Attack family | Method | Result |
| --- | --- | --- |
| Writable partial stage after SIGKILL | Executed local Darwin test | Partial file removed; no blob admitted |
| Calculated import SIGKILL | Executed local Darwin test | Unresolved intent discarded; no partial publication |
| Readonly wrong digest stage | Executed focused regression | Owner-verified mismatch discarded; import row removed |
| Publication hardlink window | Executed recovery and CAS tests | Exact inode pair reconciled; final verified |
| Symlink/unrecorded external target cleanup | Executed CAS suite on Darwin | Symlink target preserved / rejected; this is not a different-UID test |
| Different-UID regular-file cleanup | Source analysis only | `private_fd` rejects `st_uid != geteuid`; no different-UID fixture was executed |
| Hardlink cleanup | Source analysis only | `private_fd` rejects `st_nlink != 1`; no dedicated hardlink fixture was executed in this campaign |
| Unrecorded digest collision | Executed CAS suite | Existing collision preserved |
| Wrong repair input | Executed CAS suite | Corrupt final unchanged; no replacement |
| Missing recorded final | Executed `cas_repair` | Verified reimport succeeds |
| Oversized corrupt final | Executed `cas_repair` | Insufficient peak quota rejected; old bytes preserved |
| Same-UID rename/unlink race | Source analysis only | Hardening note; outside supported actor boundary |
| Power-loss durability and native ARM rerun | Not run | Existing qualification evidence is separate and not revalidated here |

## Invariant traceability

| Invariant | Enforcement | Verification | Status |
| --- | --- | --- | --- |
| Partial bytes never become an admitted blob | readonly stage, digest/size verification, durable intent | recovery SIGKILL tests; CAS suite | covered |
| Cleanup cannot follow a symlink or remove a foreign/multiply-linked inode | `NOFOLLOW`, UID/type/link-count checks | Symlink fixture plus source review; UID/link-count execution absent | partial |
| Wrong bytes cannot replace an existing final | input hash, final hash, recorded-blob gate, collision refusal | wrong-repair and collision tests | covered |
| Repair peak fits quota | physical final size plus staged size accounting | `cas_repair` quota tests | covered |
| Valid publication window can complete after crash | exact inode pair check and final verification | publication-window tests | covered |
| Recovery is bounded | paged scan and `max` bound | late-intent / prepared-GC tests | covered |
| Same-UID pathname operations are atomic against an attacker | none | no race test | unverified outside boundary; hardening note |

## Verification performed

Commands run in the dirty shared checkout on the local Darwin host, without editing production or test sources:

```text
cargo test --test recovery -- --nocapture        # 8 passed
cargo test --test cas_repair -- --nocapture     # 2 passed
cargo test --test cas -- --nocapture            # 14 passed
git diff --check -- src/cas/import.rs src/cas/recovery.rs tests/recovery.rs
```

The local runs above are Darwin execution only; they are not Linux or ARM qualification. The `foreign_files_and_links_never_collected` fixture exercises a symlink and external target, but does not change UID and therefore is not evidence of a different-UID regular-file attack. Hardlink and different-UID cases are source-verified only in this review. The full workspace suite, fault-injection power-loss matrix, and native ARM rerun were not run by this review. Later native ARM evidence, if present in repository logs, is separate evidence and not relabeled as an executed step here. No global release verdict is implied.

## Final assessment

The interrupted-import repair closes the reported failure: an initial expected digest no longer causes recovery to trust or publish a writable partial stage. The owner checks, digest/size gates, collision behavior, publication-window recovery, and physical peak quota accounting survived the focused adversarial campaign. The only material residual is the documented same-service-UID pathname race, which is outside the current private-store threat model and should remain a hardening requirement if that model changes.
