# Artifactd tar patch

This directory vendors upstream `tar` 0.4.46 (MIT OR Apache-2.0) with one
small parser hardening API. `Archive::set_extension_size_limit` bounds the
payload buffered while the standard parser preprocesses GNU long-name,
long-link, and PAX extension members. Ordinary file entries and public
`Entry::read_all` remain unchanged and streaming.

`Archive::set_extension_total_limit` applies a checked cumulative budget to
those extension payloads for each parser pass, while the per-member limit
continues to reject a single oversized extension before allocation.

The vendored PAX iterator also rejects empty interior records and records
without a terminal newline. A single terminal empty split segment remains the
normal result of a well-formed final newline.

The upstream default parser is retained instead of using `raw(true)`: default
parsing is required for correct PAX effective path/link/size handling and
following-entry framing. The limit is checked before `read_all` allocates the
extension payload. Artifactd sets a 64 KiB limit before each rootfs validation
and extraction pass. The standard parser still controls framing and checksums.

Focused tests cover oversized extension rejection, PAX path/size overrides
with a following entry, GNU long paths, malformed PAX records, empty interior
records, unterminated records, and cumulative extension exhaustion. The patch
is connected through the package-owned path dependency. Native rootfs and
dependency qualification are recorded separately from the parser tests.

## Focused evidence

The pristine 0.4.46 source was copied to a temporary standalone workspace and
the `pristine_extension` test accepted a 65,537-byte PAX extension (`1 passed`).
The patched source was copied to a separate temporary workspace and
`artifactd_extension_limits` passed all seven tests, including rejection of the
same oversized extension, correct framing after a PAX size override, and the
new malformed-record and cumulative-budget cases.
