# Scoped red-team review: CAS repair and import quotas

Date: 2026-10-05
Scope: `src/cas/import.rs`, `src/cas/recovery.rs`, `src/filesystem.rs`, and `tests/cas.rs`.
Verdict: **scoped findings resolved**. The recovery rename concern is limited to an explicitly unsupported compromised same-UID actor under the current deployment boundary; it is retained as a hardening note rather than a public API vulnerability. Native and global release gates remain separate and unapproved.

Reviewed source hashes:

- `src/cas/import.rs`: `54c62f50b82e113fa522701d9e053e99ebec07450b9a31380348a478aa704927`
- `src/cas/recovery.rs`: `76c0e71b4b93972c942ab82aeb8179d56bae62886b3daaa61052c17199228292`
- `src/filesystem.rs`: `fedaaa3b91eadd8bb69535343058c1babe461ad599308fdaf2a4dc6f70c991c1`
- `tests/cas.rs`: `f2bd6f89502d883bbc7f835b72a360cfc11efc6acf00fb4f19a8044a2d48130b`

## Finding CAS-REPAIR-001 — RESOLVED: replacement quota checks final usage, not staging peak

The previous version subtracted `replacing_size` from blob usage before checking `max_store`, which admitted a replacement whose old published blob plus new temp exceeded the quota. The frozen implementation now checks aggregate blobs plus existing temps plus prepared bytes plus the incoming temp, without subtracting the old blob (`src/cas/import.rs` SHA-256 `1a61cc22acefeed4a60e818ed418f6b91f3c32dc1362d219a45ffe2f5533a728`).

The focused CAS qualification reports two replacement cases admitted under sufficient capacity and one over-peak case rejected with the original corrupt bytes unchanged. This closes the reported quota condition for the reviewed implementation. The test source is frozen at SHA-256 `f96705929ca7f7e3f7b676781b56214165e40f6aa77bf829d7b12cdbf264c057`.

## CAS-REPAIR-002 — threat-boundary disposition: recovery pathname race

When an import intent has a verified temp and no final path, recovery calls `self.temp.rename(&record.temp, &self.blobs, digest.hex())` (recovery.rs:59-63). The absence check and rename are separate pathname operations; `rename` replaces an entry if another same-UID local actor creates the digest path between them. A similar window exists in the recorded-object replacement branch (recovery.rs:47-57). The normal import path uses `hard_link` and handles `AlreadyExists`.

The current service admits peers through `SO_PEERCRED`, uses a dedicated service UID, requires a private mode-0700 store, and holds an exclusive owner lock. Public API clients receive data/handle FDs; they do not receive a parent directory capability or a pathname mutation operation. I found no reachable distinct-UID or public-client race that can create the target between these two recovery operations. A compromised process running as the service UID can already rewrite the database and digest files directly, so that actor is outside the supported principal boundary and cannot be made safe by a post-verification pathname check alone. Existing serial collision tests prove preservation for the supported foreign-file case. If co-UID adversaries are in scope, the implementation needs an explicit no-replace/compare-and-swap publication primitive and a race test; otherwise this is documented hardening rather than a release-blocking public API finding.

## Controls that are correct in the reviewed source

- Imported bytes are streamed into an exclusive private temp, hashed, size-checked, made read-only, and directory-synced before publication.
- The normal import collision path refuses replacement unless a matching DB blob record exists and the recorded size matches the request. An unrecorded digest collision is preserved.
- Recovery likewise computes `recorded_blob` from a DB record with matching expected size before allowing replacement of an existing corrupt target.
- Publication and recovery verify the final digest/size before completing the DB mutation, and a pending import intent remains available when cleanup or recovery cannot prove ownership.
- `filesystem::read`/`owned_remove` use descriptor-relative `NOFOLLOW` opens and private UID/mode/link-count checks. These checks are useful ownership gates but do not make a later pathname rename compare-and-swap.

The reviewed tests cover serial corrupt-record repair, wrong-input refusal, unrecorded collision preservation, publish-window recovery, and basic quota limits. They do not cover the peak-quota case. Native qualification and final simplifier hashes remain pending; this report makes no claim about them.
