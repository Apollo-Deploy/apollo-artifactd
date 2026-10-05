# Bounded prepared-GC recovery qualification

Focused recovery evidence; this is not full crash-campaign or release approval.

The existing public Store recovery test now covers both an interrupted prepared-tree deletion and a completed filesystem deletion awaiting database completion, with either zero or 70 live pin records. Each case reopens the store, runs 128 calls to `reconcile(1)`, verifies the retained blob bytes on every call, and requires the prepared tree and its durable intent to disappear. A further restart and reconciliation remain idempotent. The durable crash state is constructed using the existing state fixture; this does not simulate power loss or replace native SIGKILL campaigns.

Test-audit authoring gate: the observable contract is eventual recovery after bounded root marking while pinned bytes remain intact. Resetting the root cursor on every reconciliation is a credible regression. The former zero-root cases cannot reach the multi-call root walk. This extends the existing owning recovery test and introduces no production seam.

Commands from the respective existing qualification checkouts:

```text
ssh tihan-apollo 'cd ~/artifactd-qualification-20261004 && /usr/bin/cargo test --test recovery prepared_gc_intent_reconciles_with_or_without_tree_effect -- --exact'
ssh apollo-node-01 'cd ~/artifactd-qualification-20261004 && ~/.cargo/bin/cargo test --test recovery prepared_gc_intent_reconciles_with_or_without_tree_effect -- --exact'
```

Both native Linux commands passed: 1 passed, 0 failed, 6 filtered out. The local macOS Store-boundary command also passed.

An isolated temporary source copy changed GC initialization to `Cycle::new(epoch)` on every call, discarding persisted progress. The same local test failed with exit 101 at the prepared-tree absence assertion. The temporary copy was automatically removed; production source was not altered by the control.

SHA-256 hashes verified identical locally and on both native hosts:

```text
tests/recovery.rs    cd01a5c557e8f71a1efeda3e06350c1e7ec5cfba4e379368c2fa155e8ebf6196
src/cas/recovery.rs  04c6339c4f7d83fc64b40760b656cae524f098746e11b4bdc6bdbb5568067dcb
src/gc/mod.rs        f1dddcfffd6d93883d0cd5f4988aa713c23db77e286c3643c1ca07ffd06e0d53
src/gc/prepared.rs   62755cd4c72bfc85d940c6dfa7f9433be02514c293f9c85f5bf243594aa2bf9d
```
