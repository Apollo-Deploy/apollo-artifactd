# Registry pull protection

`PULL` optionally accepts a generic `PinId`. Both a digest-pinned HTTPS fetch
and an independently verified local cache hit use the same atomic OCI graph
admission transaction as local imports. The graph and requested pin are
committed before success; `pin_id` is returned. Omitting the pin preserves an
unprotected cache import. Pin conflict, identity, capacity and epoch checks
remain owned by the shared admission transaction.

The existing authenticated registry daemon test is the primary transport owner.
It pushes a layered image to an isolated HTTPS registry, pulls it through a
protected credential FD with a pin, runs GC before preparation, restarts and
runs GC again, then reuses the local graph under a different pin. It releases
the original pin and proves the cache-created pin protects the graph before
preparation. Existing preparation, auth failure and credential/journal checks
remain in the same test. Fixture setup was extracted into test support to keep
the API owner modular; no test-only production seam was added.

Two controlled ARM negative runs restore the old unpinned effect at the
network/cache admission call sites and the cache-only call site respectively.
The successfully fetched or cached graph returns no requested pin receipt,
causing the intended assertion failure. Logs: `pull-pin-pre-fix-arm64.txt` and
`pull-pin-cache-pre-fix-arm64.txt`. Two earlier cache-control attempts hit a
connection-refused harness issue and are not semantic proof. A fresh runtime
directory alone did not resolve it. The daemon test fixture now waits for a
successful public capabilities response instead of socket-file existence,
which can precede listen. It does not remove stale or foreign sockets; daemon
ownership checks remain authoritative. The handshake uses the existing
client timeout, rather than claiming a strict 60-second startup deadline.

No production registry, service or artifact store is changed. Both host
fixtures are isolated Distribution registries. Credentials remain protected
FD input and are neither recorded in request JSON nor persisted in state.
`RED_TEAM_PULL_PIN.md` records the scoped independent review. Dedicated
service identities, actual callers, full crash/fault/concurrency campaigns,
final churn and old Zig removal remain mandatory gates.

Final native evidence: ARM `pull-pin-native-arm64.txt`; x86 production release
`pull-pin-native-x86_64.txt` followed by final test/support checks in
`pull-pin-final-x86_64.txt`. Both hosts passed authenticated HTTPS daemon tests,
normal locked workspace/all-targets tests, clippy with warnings denied, and
release/all-targets builds. Final snapshots match all 100 source/build inputs
on each host. The x86 release build precedes the test-only readiness change;
its production sources match the final source.

Commands: `cargo test --locked --test registry_api -- --ignored`,
`cargo test --locked --workspace --all-targets`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`,
`cargo build --locked --release --all-targets`. This does not rerun the older
ignored SIGKILL or long churn campaigns against the new pin API.
