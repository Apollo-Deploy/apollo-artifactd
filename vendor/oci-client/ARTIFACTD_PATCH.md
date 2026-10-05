# Maintained OCI registry client fork

This directory contains the Apollo Artifactd maintained fork of upstream `oci-client` 0.18.0 from [oras-project/rust-oci-client](https://github.com/oras-project/rust-oci-client). The upstream Apache-2.0 license and attribution are retained.

The fork preserves the upstream OCI registry, HTTP, and TLS implementation while adding bounded metadata and authentication response bodies, request deadlines, bounded secure redirects, exact bearer-realm authority policy, sanitized errors, and safer upload-location handling. These controls are configured and enforced by the Artifactd integration.

Review upstream changes before updating this fork. See the upstream `LICENSE` in this directory for redistribution terms.
