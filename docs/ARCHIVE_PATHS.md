# Canonical archive paths and OCI layout extensions

Rootfs and OCI layout import share `archive_policy` for canonical paths and
strict PAX metadata validation. The mature tar parser handles GNU/PAX framing
under per-extension and cumulative allocation limits. Artifactd does not
implement a second tar parser or unpack OCI archives to arbitrary paths.

Current-directory components are discarded before duplicate detection.
Absolute paths, parent traversal, NULs, invalid UTF-8, and excessive
length/depth reject admission. Canonical aliases cannot overwrite each other.
A canonical empty path is permitted only for a zero-sized directory; rootfs
extraction skips that directory without changing the prepared root's mode.
OCI layout directories are limited to the root, `blobs` and `blobs/sha256`,
all with zero-sized payloads. File names remain constrained to OCI layout
metadata and SHA-256 blob members.

The public rootfs regression demonstrably fails at the previous `./` guard
(`dot-path-pre-fix.txt`) and passes after normalization. Its security table
covers PAX root-file confusion, parent traversal and absolute paths; only
private staging entries may remain after rejection, for recovery. No final
prepared directory is published. Existing whiteout, DiffID and recovery tests
remain in the focused run.

`oci-extensions-pre-fix.txt` retains the old raw-parser failure for the
standard PAX-prefixed layout fixture in a temporary workspace.
The valid OCI archive fixture now uses PAX effective `./` paths and verifies
the resolved manifest. Further OCI cases cover ambiguous PAX metadata,
nonzero directory payloads and canonical duplicate layout names.
`archive-paths-focused-local.txt` records passing preparation, OCI and
recovery suites before strengthening the negative assertions. The final
OCI suite has eight tests, including the otherwise valid nonzero-directory
archive and exact duplicate-PAX guard. `oci-directory-size-pre-fix.txt`
records controlled size-guard removal admitting that malformed archive. Native normal suites, Clippy and release builds are tracked
in `archive-paths-native-{arm64,x86_64}.txt`; both completed successfully.
The production snapshots precede the test-assertion refinement.
`archive-paths-final-{arm64,x86_64}.txt` records final focused and normal
workspace suites and Clippy; both final 97-file snapshots match current
source and tests. Production source is unchanged from the release builds.
Ignored registry and real SIGKILL checks passed on both hosts, retained
separately in `archive-paths-fault-registry-*` logs. These cover HTTPS graph
and retry verification, protected credential-FD/journal/prepared facts,
interrupted imports, partial prepared GC and interrupted registry pulls.

The scoped independent review is `RED_TEAM_ARCHIVE_PATHS.md`. This change
addresses standard archive compatibility while retaining the outstanding
hardlink, absolute-link, non-UTF-8, cross-UID and immutable-consumption work.
It does not qualify all production requirements or authorize old Zig removal.
