# Focused registry admission and cache-authority checks — 2026-10-05

Two public `Registry` contracts changed:

- `PUSH` rejects a raw CAS blob, including bytes shaped like an invalid OCI
  manifest, unless the root has been admitted and its stored graph verifies.
  Rejection happens before registry credentials or transfers are used.
- Cached `PULL` still reuses a verified local digest without a network request,
  but a supplied credential provider must name the requested registry.
  Validation on this path does not clone secret values into an unused registry
  auth object.

The user-requested test-audit procedure was applied to both regressions:
each exercises the public `Registry` API,
has a credible pre-fix failure, and asserts an externally meaningful result
without a test-only production seam. The private credential file is mode 0600.

`tests/registry.rs` has two nonignored tests. The PUSH fixture imports a
schema-version-1 manifest as raw CAS content, then expects `push requires
admitted OCI root` rather than credential validation. A temporary local
negative control with only the new guard removed failed with `credential
authority mismatch`; restoring the guard passed. The PULL fixture admits a
valid local image, supplies credentials for another authority, and expects
`credential authority mismatch` with unchanged store status. A temporary
local negative control with only the authority check moved back after the
cache return unexpectedly succeeded and returned the cached graph; restoring
the check passed. Both negative controls were restored before native runs.

| Host | Checkout | Command | Result |
| --- | --- | --- | --- |
| macOS local | current workspace | `cargo test --locked --test registry` | 2 passed, 1 HTTPS fixture ignored before the no-clone refactor; focused PULL test passed after it |
| `ssh tihan-apollo` x86_64 | `/home/tihan/artifactd-cas-repair-20261005.Fb6fUB` | `cargo test --locked --test registry` | 2 passed, 1 HTTPS fixture ignored after the no-clone refactor |
| `ssh apollo-node-01-internet` aarch64 | `/home/apollo-admin/artifactd-cas-repair-20261005.GPrTLa` | `cargo test --locked --test registry` | 2 passed, 1 HTTPS fixture ignored after the no-clone refactor |

The three checkouts had identical SHA-256 for the changed inputs after the
no-clone refactor:

```text
credentials.rs fe2bc5b537ac0ef3719c4647bec41b4aaa6bd78315c34311fa634708ad895521
pull.rs      b65ea20366c52bd8ae46cd35708f7c5cf0e836eacd7069038835db72e6a947c1
push.rs      622c3f3fb1b83ddda753302f8ec17ff6013cdacf901937b34a34947cf8b6f628
registry.rs  ffea4b01a0017e489e0276e29808f2ec8ccf55878f8136404cfeb8b474be9b08
```

The remote checkouts retain an older `Cargo.lock` hash
`14f140414a34fb6b204cd0a2db3c23f1da567787a8d85923f59b3473544644a0`.
The local lock hash is
`1713a22a5408df6e03dc3f48b888550cb0306d00131f2c8e6ee4e7578996b324`;
their diff adds only the `artifactd-client-ffi` workspace package entry and
does not change resolved registry dependencies. These are focused source
checks, not coherent current-workspace native release builds.

The ARM64 host also has an owned, private Distribution registry fixture bound
only to `127.0.0.1:38029`, with its state inside a mode-0700 qualification
directory and credentials supplied by a mode-0600 file. Against this isolated
HTTPS fixture, the changed-source `https_registry_push_pull_verifies_graph_and_retries`
test passed (1 test, 7.90 seconds), followed sequentially by the daemon
`registry_api_fd_credentials_journal_and_prepared_facts` test (1 test,
5.17 seconds). Both used `--locked --ignored --exact`; the API test source
hash matched the current local file
`c075aaf69e182bf2d0f96070639717da93f4b005e347aff34bc5c7ac2afa8ce1`.
The x86_64 host used a separate owned Distribution v3.1.2 fixture bound only
to `127.0.0.1:35497`, with private mode-0700 state and a mode-0600 credential
provider. Its initial certificate carried `CA:TRUE`; the Rust client failed
before the registry observed a request, although a Python HTTPS probe
succeeded. This points to a TLS-client fixture mismatch, but the sanitized
client error did not expose the underlying cause. Reissuing the fixture
certificate with an IP SAN and no CA constraint fixed that fixture. The
changed-source
`https_registry_push_pull_verifies_graph_and_retries` test then passed (1 test,
10.97 seconds), followed by `registry_api_fd_credentials_journal_and_prepared_facts`
(1 test, 7.47 seconds). Both used `--locked --ignored --exact` and the same
private fixture; no production registry or credentials were used.

These qualify the two scoped authenticated HTTPS paths on both native
architectures. They do not qualify broad registry faults, production caller
cutover, coherent current-workspace release builds, or the independent release
gate.
