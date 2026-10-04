# Primary implementation review, round 1

Baseline: current standalone Rust source, 2026-10-04. No production completion or review convergence claim. User requirements remain fixed. Authorized Linux targets: isolated qualification directories on tihan-apollo (x86_64) and apollo-node-01 (arm64). Existing daemons/data are outside the test boundary.

Trust map: peer UID + bounded FD packet -> generic request -> durable operation intent -> streaming CAS / verified OCI -> SQLite references -> preparation publication / returned FD -> explicit lease release -> ownership checked collection.

Primary unresolved requirements before independent criticism:

- Registry client acceptance: reviewed oci-client 0.18.0 `_pull_manifest_raw` and error-body reads buffer unbounded responses. No safe bounded client integration accepted. PULL/PUSH unavailable.
- BuildKit publisher / Node / release callers remain Zig-dependent; no removal authorized by completed qualification yet.
- Full restart/disk-full/SIGKILL campaign, fuzz, 100k/20k churn and resource/latency measurement incomplete.
- Same-UID peer policy authenticates local caller but is not a cross-user deployment model. Returning owned directory FDs to the owner cannot enforce OS immutability against that UID.
- Prepared extraction rejects hardlinks and absolute symlink targets. These restrictions must not silently become a completed port of the baseline optional safe-link policy.
- SQLite structural integrity does not prove persisted reachability corresponds to rehashed OCI graphs. GC must fail closed on logical corruption.
- GC batch size bounds deleted records, but recursive reachability may scan the full database. Incremental global work is not yet demonstrated.
- Root/CAS ownership checks and exclusive lock exclude other daemon processes, not hostile code under the store owner's UID. This is not an established adversarial same-UID isolation guarantee.

Counterexamples prioritized: crash after CAS link before temp unlink; crash during recursive prepared deletion; forged descriptor platform/size; indirect symlink escape; poisoned reachability; replay Open after lease release; mutation operation history after GC; protocol truncation/excess FDs; incomplete tar terminator; quota oversubscription across prepared output.

Fixed gates require native tests, production build/audit/tree/SBOM, performance/churn, full callers and Zig removal, and two complete clean independent rounds. Readiness is not inferred from library tests or compilation.
