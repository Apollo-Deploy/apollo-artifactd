# buildd V3 client qualification

Recorded 2026-10-05. This captures the Rust `apollo-buildd` adapter and its
ignored public-protocol integration test against the independently built
Artifactd daemon. It qualifies the client/wire boundary on native Linux x86_64
and aarch64; it does not qualify the production caller or cross-UID policy.

## Source identity

The source files below had identical SHA-256 values in the local workspace and
the fresh native qualification trees
`~/artifactd-buildd-v3-qualification-20261005` on both hosts:

| File | SHA-256 |
|---|---|
| `apollo-buildd/Cargo.toml` | `8429089b498f0195a45846fb29a27791e2d166046790e498efe42ba9ce559b24` |
| `apollo-buildd/Cargo.lock` | `b784f4a633930b90c9e5e17d1c24861553d068caa0034c429c8cdb51568d1673` |
| `apollo-buildd/src/artifact/mod.rs` | `1d5d3b512fb19ed3b6c37903d01b115f1440b7e5328697ac264ad9db6712d851` |
| `apollo-buildd/tests/artifact_boundary.rs` | `ed30632bfefeb5fec760bbe80391147d63928001e2a396dce3fb68f731ab0e2f` |
| `apollo-buildd/crates/buildd-protocol/src/lib.rs` | `c3d0dd922d31b7010552ea04334a4a97b527bcae34c3969279bebc99d2af5fbd` |
| `apollo-artifactd/crates/artifactd-protocol/Cargo.toml` | `7e65454d6454d330e12e1319e06c231c3d539ef8d3d389e0c6cbe10e8b38c127` |
| `apollo-artifactd/crates/artifactd-protocol/src/lib.rs` | `c74c3a32ec9567e62ae69dd143b6f4300890dbc54d4d06692c86aa1583bfee38` |
| `apollo-artifactd/crates/artifactd-protocol/src/client.rs` | `492b0303fe4ed40d1be1bdf604548b523961a9ed14a8df3fbb4f77bfa75a79b3` |
| `apollo-artifactd/crates/artifactd-protocol/src/wire.rs` | `bb66b6948705d5c6eb89f5fa75cd932bb44448cebdfa784705655cc22d77c471` |

`apollo-buildd` depends on the standalone local `artifactd-protocol` package
at `../apollo-artifactd/crates/artifactd-protocol`; the lockfile resolves
`artifactd-protocol 0.1.0`. The client uses its V3 `client::Client`, removes
the duplicate handwritten transport, and exposes a caller-persisted operation
token plus stable pin as `ImportIntent`. Imports and registry pulls supply that
pin; the API keeps the caller-owned, read-only, private, bounded input FD checks
and exposes pin release explicitly.

## Native evidence

Both hosts used fresh source-only staging trees and separate target directories.
The test daemon and its caller ran as the same nonroot account on each host:
UID/GID `1001/1001` on `tihan-apollo` and `501/1000` on `apollo-node-01`.
Artifactd was started by the test without `--policy`, so the daemon used its
default same-UID/GID admin peer. The fixture therefore proves neither a
distinct-UID producer/consumer policy nor configured socket-group access or
per-role action denial. Production cross-UID authorization remains a separate
deployment gate.

| Host | Native target | Rust toolchain | Artifactd release binary SHA-256 | Integration test executable SHA-256 |
|---|---|---|---|---|
| `tihan-apollo` | Linux x86_64, kernel `6.12.107+deb13-amd64` | cargo/rustc `1.96.0` | `6ad9705431e8935d8642327e8bdc4f65912eb4fba601d4f5e66c72bc99bf3ab6` | `61e9430193838fe5e70a53878e51bf45b7a20c906d48915f13488252191a00b6` |
| `apollo-node-01` | Linux aarch64, kernel `6.8.0-146-generic` | cargo `1.99.0`, rustc `1.99.0` | `786febc77604c3159b8fe4e869ec84e4343707ad82fc52b2042f2fce6ea59f27` | `9c9071b9ad5474e9eb95f639d436b9872faec7064fb8b6963d2144fa480633f9` |

On x86_64, these exact commands compiled and ran the real-daemon test:

```sh
cd "$HOME/artifactd-buildd-v3-qualification-20261005/apollo-buildd"
export CARGO_TARGET_DIR="$HOME/artifactd-buildd-v3-qualification-20261005/target"
export APOLLO_BUILDD_ARTIFACTD="$HOME/artifactd-qualification-20261004/target/release/apollo-artifactd"
cargo test --test artifact_boundary --no-run
cargo test --test artifact_boundary real_artifactd_v3_fd_import_and_replay -- --ignored --exact --nocapture
```

On aarch64, the same commands used `"$HOME/.cargo/bin/cargo"` instead of
`cargo`. Both native runs compiled the Linux-gated test and reported
`1 passed; 0 failed; 0 ignored` for the explicitly selected test. The archive
FD begins at offset 7 and both successful imports leave that caller offset
unchanged while digest/config/manifest facts prove the daemon received the
complete archive. It exercises operation/pin allocation, caller-persisted
intent replay after daemon restart, SCM_RIGHTS archive import, malformed
archive rejection, foreign-store token rejection, and missing-socket rejection.
The malformed archive uses its own fresh operation token so the expected
rejection is not masked by a prior successful idempotent replay. A temporary
old-seek implementation failed the same test with the caller offset at 5120,
providing the intended negative control.

The x86_64 full `cargo test` run passed 12 tests: 2 unit tests, 8 contract
tests, and 2 source-capacity tests. It reported two ignored tests: the
real-daemon Artifactd test (run separately and passed above) and
`image_import_fd_advances_watermark_and_image_is_inspectable`, which requires
an independently running native sandboxd v2 daemon and OCI archive. Local
`cargo fmt --check` and `cargo check --lib` also passed.

## Production caller integration (2026-10-05)

The buildd production solve path now calls the V3 client. `build::solve::execute`
allocates an Artifactd import token and stable pin, persists them with
`State::begin_publication` before `IMPORT_OCI_ARCHIVE`, persists an operation ID
and verified digest/reference before optional `PUSH`, then records completion
from Artifactd's returned facts. `api::serve_with_configuration` invokes
`publication_recovery::recover` before opening the listener; recovery replays
the same durable import and push identities and never starts a new BuildKit
solve. Source inspection confirms the call path in `src/build/solve.rs`,
`src/build/publication_recovery.rs`, and `src/api/server.rs`.

On 2026-10-05, the command
`CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --locked --release --bins` passed
from source-only staging trees on native Linux x86_64 (`tihan-apollo`, Rust
1.96.0) and ARM64 (`apollo-node-01-internet`, Rust 1.99.0). The build inputs contained the
current `apollo-buildd` source and `apollo-artifactd/crates/artifactd-protocol`.
All listed changed caller files had matching local and native-staging hashes.
Their SHA-256 values are:

| Source file | SHA-256 |
|---|---|
| `apollo-buildd/src/build/solve.rs` | `eaa795652b1ef2c95f894f201686072fd06308b3911b4eda7149a965e65f8591` |
| `apollo-buildd/src/build/publication_recovery.rs` | `cf5109110c695daa07522864cece38e2bdc5578e08f560f28430044950566426` |
| `apollo-buildd/src/build/run_mounts.rs` | `31b604416131d340287d624b8591847a57d8514d529d243182c1c9dd841c74be` |
| `apollo-buildd/src/build/recovery.rs` | `f4390db2214d1fc4199a526afdc8644134c6b99a417f7f54bcdc5b5eaae14824` |
| `apollo-buildd/src/api/server.rs` | `1fb461cf349a8e885683b63fe071cdaa35a5554fa25f966ff4a5f9dc2877cf47` |
| `apollo-buildd/src/sandbox/mod.rs` | `0206fa4d2febf5ded157d78de976534b2280640df64f21974099c33371fc9e36` |
| `apollo-buildd/src/sandbox/protocol.rs` | `0f45a884c17216e4fe934263cbffef87021cea37b722cc8dd744722c9e606150` |
| `apollo-artifactd/crates/artifactd-protocol/src/lib.rs` | `c74c3a32ec9567e62ae69dd143b6f4300890dbc54d4d06692c86aa1583bfee38` |

Binary SHA-256 values:

| Host | `apollo-buildd` | `apollo-buildctl` |
|---|---|---|
| `tihan-apollo` x86_64 | `40207349b430400c3e3328d97c0ce388270017b66eb1f0582364b44faeca8347` | `c1aa483e390c8f8258663b70742274963553ab3e8bb72d03ee508f1b469121bd` |
| `apollo-node-01-internet` ARM64 | `6be3624d1dfdf57df85dae7a17464a796fa54d1884ffb558b9502fd689389cd2` | `a7fa9823abf04c1465cb33a76154cd8830e1cb155303404bf9a177cc7f4ec546` |

The first build exposed six unused/dead-code warnings in buildd's mount and
sandbox modules. I removed only the unused imports, binding, state fields,
duplicate recovery type, and superseded convenience wrappers, then rebuilt both
release binaries. The final native builds emitted no buildd compiler warnings.
This evidence establishes production-source compilation, not runtime execution,
registry publication, crash recovery, deployment, or final caller cutover. The
earlier V3 boundary integration result above qualifies the client/wire layer
only. No unit tests were added or run for this update.
