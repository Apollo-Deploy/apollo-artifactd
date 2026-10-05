# Rootfs symlink qualification

This is focused evidence for the Rust rootfs symlink policy only. It is not a release or production approval.

The public `prepare_extensions` test covers simple absolute BusyBox rewriting, literal relative targets, intermediate symlink resolution for relative and absolute targets, PAX `linkpath` dot suffixes, and escape rejection. The intermediate fixture uses `a -> dir/sub` and verifies that both forms resolve to `dir/b`; lexical normalization would instead select the root `b`.

Commands run from each qualification checkout:

```text
~/.cargo/bin/cargo test --test prepare_extensions
/usr/bin/cargo test --test prepare_extensions
```

Results on `apollo-node-01` (ARM64) and `tihan-apollo` (x86_64) were each `8 passed; 0 failed`.

An isolated temporary checkout with the dot-preserving absolute branch disabled failed the semantic regression as intended:

```text
bin/absolute was lexically normalized
left: [110, 111, 114, 109, 97, 108, 105, 122, 101, 100]
right: [108, 105, 116, 101, 114, 97, 108]
```

The temporary checkout was removed after the negative control. Current source hashes were identical on both native hosts:

```text
src/prepare/symlink.rs       38d9a7b7dfbf9e475369cddf1f0db5b58e32da31fbd32bf5ab9088b08af46772
src/prepare/layers.rs        89fd2718b9124aa8a3493ce34619946d4375458fcb88f6f5e6a3e54cb6c6133a
src/prepare/mod.rs           0699554192b2ddb2e0b25e8cc5af4d34de0522937cd58d7168f02a73354c3cd0
src/api/dispatch.rs          d756c25545b85ce3977bda351d227c9a5a518d3d5cd14ebfa1680615290d53ec
tests/prepare_extensions.rs 9dbef88f12243b82f58d4aff1958143de0f4ead2ea781db1f60d21e6d7db8959
```
