# Red-team review: archive path normalization

Date: 2026-10-05
Scope: the shared `src/archive_policy.rs` path/PAX policy used by
`src/prepare/layers.rs` and OCI archive import, plus the public preparation
tests.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_BLOCKED`. Within this scope I found no confirmed
path traversal, normalized-alias publication, root file/type confusion, or
partial prepared publication. The review is limited to rootfs archive paths;
it does not qualify OCI archive import, registry behavior, GC/recovery, Zig
removal, or the full production gate.

## Findings and evidence

The shared `archive_policy::safe_path` now rejects NUL, empty input, absolute
paths, `ParentDir`, and
excessive length/depth, while discarding `CurDir` components into a canonical
`PathBuf`. Duplicate detection runs on that canonical path before any layer
effects. Thus `./dir/file`, `dir/./file`, repeated separators, and
`dir/file` collide instead of publishing two names. The public regression
passes the `./dir/file` case and rejects the alias collision.

The canonical empty path is admitted only for an empty directory. The second
pass applies the same rule before trying to derive a filename, and does not
create a file at the prepared root. The traversal and absolute PAX-path cases
are rejected before the prepared directory is published; the test confirms
that only private `.staging` entries remain, with no final prepared artifact. The pre-fix focused log records the
expected `./dir/file` failure; the post-fix log passes.

Whiteouts still operate on the canonical parent and basename. A root
`.wh.<name>` removes only an owned root child, while `.wh..wh..opq` clears the
owned root directory. Empty, dot, and dot-dot whiteout targets remain rejected;
no path alias was found that changes the whiteout parent after normalization.

The first pass validates the complete decompressed stream, DiffID, effective
entry type/size, and normalized duplicate set before applying whiteouts. The
second pass repeats normalization, PAX validation, entry/output budgets, and
stream verification, so a path/type change between passes is rejected. CAS
publication and prepared staging remain outside this patch; no new partial
publication path was observed here.

## Remaining qualification

The scoped archive-path qualification now has ARM64 and x86_64 native logs:
the strengthened OCI archive fixture reports 8 passing tests on each host,
the preparation fixture reports 5 passing tests, and the normal workspace,
clippy, and release builds completed successfully. The public fixture rejects
the nonzero `blobs` directory prefix with the exact policy error, while the
pre-fix control demonstrates that removing the size guard admits it. The
authenticated registry graph/retry, registry API credential/journal, and
registry SIGKILL tests also pass on both hosts in the dedicated fault logs.

These results strengthen this scoped review but do not qualify all archive
format compatibility, global recovery/churn/performance requirements, caller
cutover, or removal of old Zig paths. The overall release gate remains
blocked.
