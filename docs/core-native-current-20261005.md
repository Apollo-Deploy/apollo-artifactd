# Current native core qualification — 2026-10-05

Both `tihan-apollo` (x86_64) and `apollo-node-01` (aarch64) passed:

```
cargo test --locked --workspace --lib --bins --tests
cargo build --locked --release --bins
```

Each run has 57 passing tests, zero failures, and 11 ignored tests across 29 targets. Ignored privileged/fault/registry tests are not qualified by this ordinary suite. Complete output is retained in `core-current-{x86_64,arm64}-20261005.log`; the 125-input source snapshots are `core-current-{x86_64,arm64}-snapshot-20261005.json`.

Those snapshots predate two formatting-only updates in `src/prepare/symlink.rs` (import ordering) and `tests/prepare_extensions.rs` (assert formatting). Both files were subsequently synchronized to both hosts. No behavior changed. The public churn example is qualified separately after its current source is frozen; it is excluded from the ordinary core snapshot. No global production/release approval is implied.
