# Bounded standard tar extensions

Rootfs preparation uses the mature tar 0.4.46 parser's standard GNU/PAX
interpretation. A narrowly vendored mechanism limits each buffered extension
to 64 KiB and cumulative extensions to 64 MiB per layer. Both passes configure
these limits. Ordinary entries remain streaming; no tar framing parser or
unconstrained unpack routine is implemented in artifactd.

Artifactd validates every PAX record before interpreting effective paths or
applying effects. Malformed framing, invalid unsigned decimal size/uid/gid,
duplicate keys and oversized metadata reject preparation. The parser rejects
empty interior records and missing final newlines. Unknown unique PAX keys
are parsed but do not add filesystem metadata effects.

Validation and extraction independently enforce entry count, entry size,
output and decompressed byte bounds, tar termination and DiffID. The first
pass's decompression budget spans the whole image. The extraction pass is
bounded to the already verified layer length and must reproduce its entry
count/output accounting. Failed verification leaves no published prepared
rootfs. Whiteouts remain applied in layer order after full initial validation.

## Evidence

`prepare-extensions-pre-fix.txt` records valid GNU, PAX-path and PAX-size
fixtures rejected by the old raw-entry preparation. The PAX-size fixture
overrides the physical header size and verifies the following file, so a raw
framing prepass is not substituted for standard parsing.

`pax-policy-pre-fix.txt` records malformed, invalid numeric and duplicate
metadata accepted before the policy repair. `decompression-image-pre-fix.txt`
is a controlled negative run restoring the old per-layer budget: two small
layers exceed the combined budget but were accepted. The source was restored
before subsequent checks. `tar-prepare-final-local.txt` records focused local
checks; the separate native final logs identify Linux qualification.

Both native Linux architectures passed the three preparation regressions, normal workspace all-targets suite, warning-free Clippy, and release all-targets build. The `tar-final-native-{arm64,x86_64}-snapshot.json` files matched all 97 source and qualification files at that run; the subsequent registry benchmark receipt-check repair changes only the example. The snapshots identify debug daemon/CLI binaries, while performance logs identify release runners. Ignored fault tests and broader release gates require separate evidence.

The vendored parser tests cover individual/aggregate limits, standard size
framing, GNU paths, malformed records and newline boundaries. Upstream code,
licenses and version are preserved; [patch provenance](../vendor/tar/ARTIFACTD_PATCH.md)
describes the mechanism changes. This fork requires ongoing security review.

Hardlinks, absolute symlink targets, non-UTF-8 names and some OCI-layout
archive extensions remain unsupported. Cross-UID consumption and owner-UID
immutability also remain required work. These limits prevent full rootfs and
release approval; they are not deferred out of the user's cutover scope.
