# Incremental GC walk module split — 2026-10-05

The durable OCI graph walk was split so no non-generated Rust source file
exceeds the project's 500-line hard limit. `src/gc/walk.rs` now holds the
serialized walk state and graph admission helpers (267 lines);
`src/gc/walk/engine.rs` holds the Store-backed walk advancement and graph
verification methods (261 lines). The split preserves the existing budgets,
serialized state shape and control flow. It changes method visibility only so
the parent `gc` module can continue to call the walk engine.

The current staged source built both production binaries on both native Linux
hosts with:

```text
CARGO_PROFILE_RELEASE_DEBUG=0 cargo build --locked --release --bins
```

| Host | Native target | Toolchain | Result | `apollo-artifactd` SHA-256 | `apollo-artifactctl` SHA-256 |
|---|---|---|---|---|---|
| `tihan-apollo` | Linux x86_64 | Cargo/rustc 1.96.0 | release build passed in 43.48s | `71003ad591dbb736101b0f811fcc559eb762940c483cab9bdccc54b5ed096601` | `5cc21b5da4fa40dc936130b1c610484c4148b35ab5f7f6df6e33f1cc728bb55e` |
| `apollo-node-01-internet` | Linux aarch64 | Cargo/rustc 1.99.0 | release build passed in 14.96s | `4f96024fbc87971745ee980dba464934fb81c843fed27c5a7279d090a126ff4c` | `a28d8e48bbed427ab7a951852a93fddd18f38f67058f5f465f050e727f8010ee` |

Both binaries exited successfully with `--help`. The Cargo manifest, lockfile,
and both changed Rust files had matching SHA-256 hashes in the local source and
both staging trees. No test targets were built or run. This establishes that
the refactored package compiles natively on both architectures. Each exact
build then started as the ordinary SSH user with fresh private store and
runtime directories; `apollo-artifactctl` successfully called `CAPABILITIES`
and `STATUS` over the local Unix socket. Both replies reported protocol v3,
`SO_PEERCRED`, `SCM_RIGHTS`, and `production_qualified: false`; status showed
schema 3 with zero blobs and bytes. Scratch stores were removed afterward.

No test targets were built or run. This proves current-source native binary
compilation and minimal daemon protocol operation; it does not qualify
incremental GC behavior or close the wider service release gates.
