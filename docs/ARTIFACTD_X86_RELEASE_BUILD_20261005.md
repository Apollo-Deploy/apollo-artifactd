# Current x86_64 artifactd production-binary build — 2026-10-05

The current source-only package built both production binaries on native
`tihan-apollo` Linux x86_64:

```text
uname -m: x86_64
cargo: cargo 1.96.0 (30a34c682 2026-05-25)
rustc: rustc 1.96.0 (ac68faa20 2026-05-25)
command: CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --locked --release --bins
result: Finished release profile (optimized) in 2m 29s
```

The source-only staging tree and target directory were under `/dev/shm` because
the host's `/tmp` had only 207 MiB free. The workspace contained the package
manifest and lockfile, `src/`, `crates/`, and the vendored database, OCI client
and tar dependencies. A deterministic digest over all 226 files in those build
inputs was identical in the workspace and staging tree:

```text
6ccd586e912e95856711c3d8f73b1a620f26f308dac32c0c4d92f37f28632259
```

The resulting binary hashes are:

| Binary | SHA-256 |
| --- | --- |
| `apollo-artifactd` | `79139371f7be9c025a79da64d515520d3bc0d3001d6d14b023c73b15f5a85081` |
| `apollo-artifactctl` | `5cc21b5da4fa40dc936130b1c610484c4148b35ab5f7f6df6e33f1cc728bb55e` |

Both binaries also exited successfully with `--help`. This record covers
production-bin compilation and CLI startup. A later isolated daemon startup
and `CAPABILITIES`/`STATUS` exchange is recorded in
[the native service smoke record](ARTIFACTD_NATIVE_SERVICE_SMOKE_20261005.md).
No test targets were built or run for this evidence. Registry operations,
crash/restart behavior, all-target current-x86 qualification, and final
Zig/caller cutover remain separate gates.
