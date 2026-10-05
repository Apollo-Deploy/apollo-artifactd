# Maintained tar parser patch

This directory vendors upstream `tar` 0.4.46 (MIT OR Apache-2.0) with bounded GNU/PAX extension processing. `Archive::set_extension_size_limit` caps each buffered extension and `Archive::set_extension_total_limit` caps cumulative extension payloads for a parser pass. Ordinary entries and `Entry::read_all` remain streaming.

PAX parsing rejects empty interior records and records without a terminal newline. The upstream default parser remains in use so effective PAX paths, links, sizes, framing, and checksums follow standard parser behavior. Artifactd applies its own path, link, and output policies after parsing.

Upstream source and license notices are retained. Review upstream changes before updating this fork.
