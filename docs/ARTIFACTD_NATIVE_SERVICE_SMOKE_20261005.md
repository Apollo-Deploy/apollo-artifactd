# Native artifactd release-binary and service smoke — 2026-10-05

Current-source production binaries built on both native Linux hosts with
`CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --locked --release --bins` from
source-only staging trees. The x86_64 build used `/dev/shm` because `/tmp` was
nearly full; its detailed command, input digest and source hashes are in
[the x86 build record](ARTIFACTD_X86_RELEASE_BUILD_20261005.md). The 226-file
build-input digest matched on both hosts:

```text
6ccd586e912e95856711c3d8f73b1a620f26f308dac32c0c4d92f37f28632259
```

| Host | Native target | Rust toolchain | `apollo-artifactd` SHA-256 | `apollo-artifactctl` SHA-256 |
|---|---|---|---|---|
| `tihan-apollo` | Linux x86_64 | Cargo/rustc 1.96.0 | `79139371f7be9c025a79da64d515520d3bc0d3001d6d14b023c73b15f5a85081` | `5cc21b5da4fa40dc936130b1c610484c4148b35ab5f7f6df6e33f1cc728bb55e` |
| `apollo-node-01-internet` | Linux aarch64 | Cargo/rustc 1.99.0 | `a8830e6a46d3243474f886a83b4144668c0cb66170527836bcbe61acd7b09773` | `a28d8e48bbed427ab7a951852a93fddd18f38f67058f5f465f050e727f8010ee` |

On each host, the daemon ran as the ordinary SSH user with a new mode-0700
store and socket directory in `/dev/shm`. The production CLI connected over
the Unix socket and received successful `CAPABILITIES` and `STATUS` replies.
Both reported protocol version 3, `SO_PEERCRED`, `SCM_RIGHTS`, Linux amd64 and
arm64 support, and `production_qualified: false`; STATUS showed schema 3 with
zero blobs and bytes. The daemon was terminated and the isolated smoke
directory removed. No existing store or service was used.

This verifies current-source native binary compilation and minimal local
daemon startup/protocol operation. It does not qualify imports, OCI, registry,
crash recovery, multi-client behavior, deployment policy, or the full
production cutover. No unit-test target was built or run for this evidence.

## Current workspace rebuild — 2026-10-05

I repeated the build and service smoke against a source archive containing the
current `Cargo.toml`, `Cargo.lock`, `crates/`, `src/`, and `vendor/` trees. The
archive SHA-256 was identical after transfer to both native hosts:

```text
e094970f08a66bbbeb919ca47f51158840d429c5ad642212fde398e76c12a6dc
```

Both builds used `CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --locked --release
--bins`; the x86_64 build set `CARGO_BUILD_JOBS=2` and put `TMPDIR` and
`CARGO_TARGET_DIR` on persistent home storage because the host `/tmp` tmpfs was
full. The initial x86_64 attempts failed for lack of `/tmp` space; the build
below is the successful retry.

| Host | Toolchain | `apollo-artifactd` SHA-256 | `apollo-artifactctl` SHA-256 | Result |
|---|---|---|---|---|
| `tihan-apollo` | Cargo/rustc 1.96.0 | `8111bbc1781afef21e6bb3a14b3599380f8eb20bc338b30d9f807a42b5c2947a` | `5cc21b5da4fa40dc936130b1c610484c4148b35ab5f7f6df6e33f1cc728bb55e` | Release build and both `--help` checks passed |
| `apollo-node-01-internet` | Cargo/rustc 1.99.0 | `a1506b148a71982970db1b272d628e60cc3c51884ee35f3c72de3f632df8953e` | `a28d8e48bbed427ab7a951852a93fddd18f38f67058f5f465f050e727f8010ee` | Release build and both `--help` checks passed |

On each host, a fresh daemon ran as the SSH user from a mode-0700 store and
socket directory in `/dev/shm`. `CAPABILITIES` returned protocol v3,
`SO_PEERCRED`, `SCM_RIGHTS`, private-FD registry credentials, and linux/amd64
plus linux/arm64 support. `STATUS` returned schema 3 with zero blobs and bytes.
Both responses report `production_qualified: false`. The temporary service
directories were removed after the smoke. No unit-test target was built or
run. Authenticated registry transfer was not rerun because the isolated
registry listener and credential provider were unavailable on these hosts.

## Unix-socket CAS and OCI lifecycle smoke — 2026-10-05

Using those same current release binaries, I exercised the public daemon API
on both native Linux hosts with fresh mode-0700 stores and sockets under
`/dev/shm`. Blob bytes were passed through the CLI's FD interface. Import,
duplicate import, and VERIFY agreed on
`sha256:4b872a9d...d2bd06250` (38 bytes); the wrong-digest import failed and
STATUS retained exactly one blob. DOCTOR reported clean database integrity,
verified live graphs, and a valid operation journal.

I then imported a standard OCI archive with an index containing linux/amd64
and linux/arm64 zero-layer images. IMPORT_OCI_ARCHIVE selected and verified
the amd64 graph; RESOLVE selected the distinct arm64 manifest. A graph lease
was created, PREPARE published the amd64 rootfs, OPEN_PREPARED returned it as
an FD, and STATUS reported the five verified OCI graph blobs. Releasing the
lease, unpinning, and running GC collected all five; DOCTOR remained clean.
Both hosts returned matching digests and lifecycle results.

This is a direct service-level smoke, not full OCI extraction or adversarial
archive qualification: the fixture has no layers. It does not close registry
transfer, interruption, multi-client contention, or the full release gates.
No unit-test target was created, built, or run for this evidence.
