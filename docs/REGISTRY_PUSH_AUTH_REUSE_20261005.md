# Authenticated manifest read-back after PUSH — 2026-10-05

After uploading an OCI manifest, Artifactd verifies the immutable digest
reference by fetching it back. That verification now reuses the configured
registry credentials instead of attempting an anonymous fetch. The optional
tag read-back already used those credentials.

The current production binaries built with
`CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --locked --release --bins` on
native Linux x86_64 (`tihan-apollo`) and ARM64
(`apollo-node-01-internet`). Both daemon and CLI `--help` commands exited
successfully on each host. No test targets were built or run for this change.

The local source and both remote build trees had matching hashes:

| Input | SHA-256 |
|---|---|
| `Cargo.toml` | `ad81a6c3c97222b2d2a46ffcf889467f387694a3173882f15af7f2f79b88c4e0` |
| `Cargo.lock` | `c06bbc3bfc2be2ce121d3bf0940903432c5ade61b44b40337c7a3a5179740f9b` |
| `src/registry/push.rs` | `1ae7cd7b211d5b85461b66cf49889275a54c37849f0928c7ad82ae1431e0d1e2` |

| Host | `apollo-artifactd` SHA-256 | `apollo-artifactctl` SHA-256 |
|---|---|---|
| `tihan-apollo` x86_64 | `8111bbc1781afef21e6bb3a14b3599380f8eb20bc338b30d9f807a42b5c2947a` | `5cc21b5da4fa40dc936130b1c610484c4148b35ab5f7f6df6e33f1cc728bb55e` |
| `apollo-node-01-internet` ARM64 | `a1506b148a71982970db1b272d628e60cc3c51884ee35f3c72de3f632df8953e` | `a28d8e48bbed427ab7a951852a93fddd18f38f67058f5f465f050e727f8010ee` |

This is compile and startup evidence only. An authenticated registry transfer
was not rerun for this patch; complete registry fault and release
qualification remain open.
