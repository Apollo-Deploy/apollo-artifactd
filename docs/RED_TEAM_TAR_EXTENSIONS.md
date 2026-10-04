# Red-team review: tar extension limits

Date: 2026-10-04
Scope: uncommitted vendored `tar` 0.4.46 changes and `src/prepare/layers.rs`.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_BLOCKED`. The previously reported malformed-PAX
fail-open is repaired in the current source. I found no extension-buffer
allocation bypass or path/link escape in the reviewed implementation. The
tar-specific review does not qualify the broader release requirements
(registry, crash/fault campaigns, global GC, caller cutover, and
performance/churn gates).

## Prior finding disposition

### Resolved: malformed and ambiguous PAX metadata

`src/prepare/extensions.rs` now iterates every PAX record with `?`, rejects
empty or overlong keys, rejects duplicate keys, and requires ASCII-decimal
`size`, `uid`, and `gid` values that parse as `u64`. Both validation and
extraction passes invoke this policy before using effective paths, links, or
sizes. The vendored iterator now rejects blank, malformed, and unterminated
records rather than silently skipping them.

The new artifactd regression covers malformed, blank, unterminated, invalid
numeric, duplicate, and oversized metadata, and the standard GNU/PAX/PAX-size
fixtures continue to pass. The pre-fix evidence records acceptance of the
malformed, invalid-size, and duplicate cases; the current focused test rejects
all six cases before filesystem publication.

## Reviewed defenses

- `set_extension_size_limit(64 * 1024)` is set before both validation and
  extraction passes. `read_extension` checks the declared `u64` size before
  calling `read_all`; `read_all` only preallocates up to 128 KiB and then
  streams. I found no cast or `Read::take` path that bypasses this check.
- The standard parser is used instead of `raw(true)`, so PAX effective size is
  used for framing and GNU/PAX path and link overrides reach artifactd's safety
  checks. The pre-fix and post-fix `prepare_extensions` evidence reproduces
  the framing failure and its correction for GNU, PAX, and PAX-size fixtures.
- Effective paths are checked after overrides, link targets are checked for
  root escape, duplicates are checked after effective-path resolution, and
  device/hard-link/special entries are rejected.
- GNU sparse parsing validates block order, overlap, overflow, real size, and
  physical entry size. The first pass accounts for logical `entry.size()` and
  the second pass requires copied bytes to equal that size; no sparse-specific
  escape was reproduced.
- A three-extension-per-member shape is bounded by the parser's one-long-name,
  one-long-link, and one-local-PAX checks. Each extension is independently
  limited; the vendored aggregate extension limit now bounds all buffered
  extension payloads in each pass. The extraction pass repeats both limits.

## Qualification gaps

The previously noted second-pass budget gap is repaired. Both passes now
repeat entry count, logical output, PAX validation, extension limits, and
DiffID/terminator verification, and require matching count/output totals.
`VerifiedReader` also applies a cumulative decompressed-byte budget across the
whole image rather than resetting it for every layer. The new focused negative
test reproduces the old per-layer-budget acceptance and passes with the global
budget.

The focused tests passed locally:

```text
7 passed: artifactd_extension_limits; 3 passed: prepare_extensions
```

The scoped tar qualification now has completed ARM64 and x86_64 evidence:
normal workspace suites, `clippy -D warnings`, and release all-target builds
completed successfully on both hosts, with matching 97-file source snapshots
(`docs/tar-final-native-arm64.txt` and
`docs/tar-final-native-x86_64.txt`). The tar performance and ARM idle receipts
are also recorded, including the corrected private-runtime idle measurement.
This confirms the tar/rootfs scope only; it does not claim the full release
gates, registry qualification, caller cutover, removal of old Zig paths, or
overall production completion.
