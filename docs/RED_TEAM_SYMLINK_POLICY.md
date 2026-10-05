# Rootfs symlink policy review

Scoped review of `src/prepare/symlink.rs` and its layer extraction callers.
This is not a complete artifactd release approval.

## Finding

The current `safe_target` lexically normalizes `.` and `..` in relative and
absolute targets before materializing them. That changes POSIX symlink
semantics when an intermediate component is itself a symlink. For example, a
link at `x/link` with target `a/../b` resolves through `x/a` before applying
`..`; the implementation rewrites it as `b` and skips that intermediate
resolution. An absolute target such as `/a/../b` has the same issue when
`/a` is a rootfs symlink. It also discards trailing slash and terminal `.`
semantics: `/bin/busybox/` and `/bin/busybox/.` are rewritten as `busybox`,
losing the requirement that the resolved target be a directory.

All intermediate symlink targets may remain inside the rootfs, so this is not
currently a demonstrated host escape. It is a correctness/integrity issue:
the prepared tree can point at a different file than the OCI layer specifies.
Preserve relative target text after checking that its component walk cannot
escape the root, and handle absolute targets by converting the root anchor
without collapsing intermediate `..`, terminal `.`, or trailing-slash
semantics; alternatively reject targets whose `..` follows a potentially
symlinked component. Add public tests that resolve the link payload through a
safe intermediate symlink (rather than only checking `read_link` text), plus
relative/absolute `a/../b` and raw absolute targets with trailing slash and
terminal `.`. The negative controls must fail against the current
implementation for the intended semantic reason and then pass after repair.

The format v2 identity prevents reuse of v1 prepared records. No separate
cycle escape was found; symlink cycles should still be bounded by consumers'
`ELOOP` behavior and need a qualification test if cycles are intentionally
accepted.

## Closure disposition

The current source preserves literal relative targets, retains dot and
trailing-slash semantics for absolute targets that need them, and validates
root escape without collapsing intermediate components. Public tests now
resolve through a safe intermediate symlink for both relative and absolute
`a/../b` payloads and cover a PAX `linkpath` containing `/bin/busybox/.`.
The isolated pre-fix normalization control fails for the intended resolved
payload mismatch, while the current implementation passes focused native
qualification on both ARM64 and x86_64 (8/8 cases each). The scoped finding
is closed, pending only preservation of the supplied native evidence in the
final qualification record.
