# Current-workspace native qualification snapshot — 2026-10-05

The local workspace and fresh, private source snapshots on
`ssh tihan-apollo` (x86_64) and `ssh apollo-node-01-internet` (aarch64)
contained the same 172 Cargo manifest/lock, crate, source, test, example, and
vendored files. The sorted path/content SHA-256 aggregate was
`aed9b479920deeda2e82e63eee2291c4378b2de7a08f858ed3a7b5e3a7bd83b1`
on all three machines. This aggregate excludes documentation and generated
build outputs. The current `Cargo.lock` includes `artifactd-client-ffi`.

| Host | Fresh source snapshot | Native toolchain | Current-workspace checks |
| --- | --- | --- | --- |
| x86_64 | `/tmp/apollo-artifactd-current-20261005.kFPlri` | Cargo/Rust 1.96.0 | `cargo test --locked --workspace`: 68 passed, 0 failed, 11 ignored across 34 test-result groups; `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed; `cargo build --locked --release --workspace --all-targets`: passed |
| aarch64 | `/tmp/apollo-artifactd-current-20261005.IsaymM` | Cargo 1.99.0, Rust 1.99.0 | The same ordinary suite, warning-free Clippy, and release all-targets build passed |

The x86 ordinary suite initially failed during linking because the host had
only 88 MiB left on its root volume (`No space left on device`). Two older,
owned generated Cargo `target` directories were removed, leaving their source
and logs intact. A single retry with two build jobs passed. Before the x86
release build, the current snapshot's generated debug target was also removed
to free 5.4 GiB. The x86 release build then completed in 13 minutes 52
seconds; ARM completed in 4 minutes 13 seconds. These are build wall times,
not artifact throughput measurements.

Retained logs: [x86 tests](current-workspace-x86_64-tests-20261005.log),
[x86 Clippy](current-workspace-x86_64-clippy-20261005.log),
[x86 release](current-workspace-x86_64-release-20261005.log),
[ARM tests](current-workspace-arm64-tests-20261005.log),
[ARM Clippy](current-workspace-arm64-clippy-20261005.log), and
[ARM release](current-workspace-arm64-release-20261005.log).

The normal suite ignores the authenticated HTTPS fixture tests. Both native
hosts separately passed the direct registry store and daemon credential-FD
tests against owned loopback HTTPS fixtures with current production source;
see [focused registry evidence](REGISTRY_ADMISSION_AUTHORITY_NATIVE_20261005.md).
The ARM64 current-workspace snapshot itself was subsequently rerun against
its owned loopback HTTPS fixture: the exact ignored store test and then the
daemon API test each passed (`1 passed; 0 failed`), sequentially, with
`Cargo.lock` SHA-256
`1713a22a5408df6e03dc3f48b888550cb0306d00131f2c8e6ee4e7578996b324`.
The retained [store](registry-current-workspace-arm64-store-20261005.log) and
[daemon API](registry-current-workspace-arm64-api-20261005.log) logs have
SHA-256 `2ae34cb66afc4dfcd32e7ab8b1898091ed853312696c9836f75a8f3d9ac14b98`
and `1276d4de46cfa8cd274e174a1fdbc6acbcd12fe7fb5daf1e6fd5304fb67fae59`,
respectively. The x86 current-workspace snapshot was not rerun against a
registry fixture while its available disk space was below 5 GiB and its
independent long churn campaign was active.
These checks do not complete the production caller cutover, Zig removal,
fault/churn/performance matrix, or independent release red-team gate.
