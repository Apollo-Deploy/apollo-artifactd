# Node Artifact caller cutover review

**Scope.** Read-only review of the current `apollo-node` Artifact caller and
the existing BuildKit receipt boundary. This is an implementation migration
map, not a release approval. The caller cutover is currently incomplete: the
Node executable still links the Zig `apollo-artifact` package and still owns a
second CAS/OCI/registry composition.

## Evidence inspected

The current files were inspected at the following paths and hashes:

| File | SHA-256 |
|---|---|
| `APIs/apollo-deployment-api/apollo-node/src/workload/artifact_application.zig` | `3a30802b702ddf622d55aad4127f7a102b314800bf7afa311ee5b083ccb9288d` |
| `.../artifact_binding_store.zig` | `438d41653961b4abbaf4612812e904ec97ac290a72b3d0e7c87f5a3b8d543d3d` |
| `.../artifact_identity.zig` | `78c3e45bce32f165d4130745297a40547ef6fefc27a7106d200887930ae22d73` |
| `.../artifact_integration.zig` | `bc52456b191dd9599cb047977f034076cad9006f3597309bd879c523b3ab3a8a` |
| `.../artifact_runtime.zig` | `a530919444626db1a3003211287183b2ceb29d6fa7ddbce8a104b5fe3bcbca1b` |
| `.../artifact_reconcile_owner.zig` | `21ea8fac70ce2108a008369c7ba136d1ec61fc03638674cbae9dd7bbf94f141c` |
| `.../apollo-node/build.zig` | `1c6bc460ba62d91bd798633fe72293ba556392f06506849543fc9166c17e16ab` |
| `.../apollo-node/build.zig.zon` | `0271ff2d111a34752ba5b1603cdce533f65f51416a7df1725794b608d478d3bc` |

The receipt implementation inspected is
`crates/apollo-buildkit-publisher/src/{receipt,receipt_facts,verification}.rs`.
Its current `ArtifactReceipt` still contains BuildKit-specific
`build_attempt_id`, `source_snapshot_id`, and optional source bundle fields.
Those fields belong in the publisher/build system. They must not be added to
artifactd's generic protocol.

## Current call graph and cutover status

| Existing behavior | Current implementation | Required disposition |
|---|---|---|
| Store/CAS initialization and limits | `artifact_runtime.zig:18-60` constructs `artifact.Store`, calls `ensure`, and injects it into `artifact.Node` | **REPLACED_BY_BETTER_RUST_IMPLEMENTATION**: artifactd is the only store; Node keeps no artifact root or Store pointer |
| Registry configuration and bearer handling | `artifact_integration.zig:31-140` owns endpoint, repository, token and `artifact.remote.Registry` | **REPLACED_BY_BETTER_RUST_IMPLEMENTATION**: artifactd registry credentials/provider stay in artifactd; Node receives only verified facts/FDs |
| Digest parsing/conversion | `artifact_identity.zig:11-69` aliases `apollo-artifact` digest types and remote references | **PORTED_TO_RUST**: use `artifactd_protocol::{ArtifactDigest,ManifestDigest,...}` through the Rust client; any remaining Zig representation is a fixed 32-byte opaque value, never an Artifact type |
| OCI acquire/pull | `artifact_application.zig:447-450` builds an immutable reference and calls `artifact_node.acquire` | **PORTED_TO_RUST**: `PULL`/`ENSURE_LOCAL` with a digest-pinned reference and platform |
| OCI graph verification | `artifact_application.zig:144-153` calls `artifact_node.verifyGraph`; `refreshPins` calls `recover` at `279-289` | **PORTED_TO_RUST**: `VERIFY`/`RESOLVE`, then `PIN`/lease reconciliation in artifactd |
| Runtime config extraction | `artifact_application.zig:450-456` calls `artifact.inspectRuntimeConfig` and interprets user/env/command/working directory | **PORTED_TO_RUST** for parsing and verified facts; **REPLACED_BY_BETTER_RUST_IMPLEMENTATION** for transport. Node retains only workload policy checks (platform, user, command/env overrides) |
| Rootfs preparation | No independent Node preparation; current Node passes an image reference to MicroSandbox | **PORTED_TO_RUST**: `PREPARE` plus `OPEN_PREPARED`; Sandboxd consumes the immutable FD/opaque handle |
| Binding lifecycle | `artifact_binding_store.zig` records workload ID/generation/phase and manifest digest | **PORTED_TO_RUST boundary / RETAINED Node policy**: keep this Node-owned generation fencing, but replace the Zig Artifact digest type with bytes and persist opaque `PinId`, `LeaseId`, and `PreparedArtifactId` |
| Crash pin recovery | `artifact_application.zig:103-117,279-289` calls Zig `recover(active, max_pin_entries)` | **PORTED_TO_RUST**: on startup enumerate bindings, verify each manifest, and reconcile artifactd pins/leases transactionally; never infer release from desired-state omission |
| Immutable runtime image reference | `artifact_application.zig:469-475` calls `immutableImageReference` and stores it in desired state | **INTENTIONALLY_REMOVED** from the Node Artifact seam. A registry-qualified tag/reference is not the prepared artifact handoff. Desired state must carry the verified manifest identity and Sandboxd prepared handle according to the Sandboxd contract |
| Synthetic Artifact Store/Node tests | `artifact_application.zig:694-1198` directly constructs Zig Store/Node and imports test OCI bytes | **INTENTIONALLY_REMOVED**. Replace with an authenticated daemon fixture/public protocol qualification; no production-only test seam |
| Zig Artifact dependency | `build.zig.zon:14` and six `addImport("apollo-artifact", ...)` calls in `build.zig:46,58,115,129,142,156` | **INTENTIONALLY_REMOVED** after caller qualification. No sibling Artifact path, package, importer, or protocol remains in any production/test build graph |

## Concrete Rust-owned interface

The existing protocol is already the correct ownership direction: version 3,
Unix `SOCK_SEQPACKET`, `SO_PEERCRED`, and `SCM_RIGHTS`. The migration should
add/use a typed Rust caller library around `artifactd_protocol::client::Client`
and should not duplicate JSON framing or protocol structs in Zig.

The minimal caller sequence is:

1. `PULL { reference, platform, pin }` (or `ENSURE_LOCAL` for an already
   digest-pinned manifest). Accept only returned `ArtifactDigest`,
   `ManifestDigest`, config/layer descriptors and platform facts.
2. `RESOLVE`/`VERIFY` the returned manifest and inspect the verified config
   facts. Node applies only its workload policy: supported platform, user,
   command, environment, working directory and resource limits.
3. Create a lease with the caller's authenticated identity and persist the
   returned `LeaseId` alongside the Node binding. Persist `PinId` and
   `PreparedArtifactId` as opaque values, never as registry paths.
4. `PREPARE { digest, platform }`, then `OPEN_PREPARED { id, lease }`. Pass
   the returned FD/opaque handle to Sandboxd. No customer command, registry
   pull, OCI parse, or rootfs extraction occurs in Node.
5. On submit/release/recovery, use the existing Node binding state machine as
   the lifecycle fence. Complete `UNPIN`/`LEASE_RELEASE` only after Node has
   proven the exact workload generation absent. Reconciliation must be
   idempotent and must preserve uncertain operations for artifactd recovery.

If Node must remain a Zig process, the only acceptable bridge is a small
Rust-owned C ABI client compiled from the artifactd workspace. The ABI should
expose opaque client/operation/FD handles and length-delimited generic facts;
it must call the same Rust protocol client and kernel peer checks. It must not
launch `artifactctl`, spawn an importer, link `apollo-artifact`, or implement a
second wire format. If Sandboxd already has a native prepared-rootfs handoff,
prefer moving this adapter there and have Node pass only the opaque prepared
handle.

## File ownership migration

* `artifact_application.zig`: remove `artifact_node`, registry endpoint and
  repository fields. Replace `acquire`, `verifyGraph`, `inspectRuntimeConfig`,
  `recover`, `immutableReference`, and `immutableImageReference` with calls to
  the typed Rust client adapter. Keep workload policy and generation fencing.
* `artifact_binding_store.zig`: keep Node-owned phases and writer/atomic state,
  but make the manifest a `[32]u8`/canonical generic digest and add opaque pin,
  lease and prepared IDs. It must not import `artifact_integration.zig`.
* `artifact_identity.zig`: delete the `apollo-artifact` aliases. If the Node
  wire still requires 64 lowercase hex, retain only allocation-free formatting
  and strict SHA-256 validation against the Rust-returned digest.
* `artifact_integration.zig`: delete it, or reduce it to the FFI/client adapter
  with no Store, Registry, OCI, limits, or remote types. No `@import("apollo-artifact")`.
* `artifact_runtime.zig`: stop creating `/var/lib/apollo-node/artifact-store`
  and the Zig Store/Node. Keep only Node binding/control roots if still owned
  by Node; artifactd owns its private state root and socket.
* `artifact_reconcile_owner.zig`: retain snapshot/binding ownership checks, but
  verify generic manifest facts via the client and pass prepared handles to the
  external Sandboxd adapter.
* `apollo-node/build.zig`/`.zon`: remove the sibling dependency and every
  `apollo-artifact` import from release, executable, tests, stability and
  benchmark modules. Tests use the real authenticated artifactd fixture or a
  protocol-level fake, never a linked Zig Artifact Store.
* BuildKit publisher receipts: keep the receipt’s build/source provenance in
  `apollo-buildkit-publisher`; construct it from artifactd generic facts and
  verify it with `RESOLVE`. Do not put BuildAttempt/SourceSnapshot concepts in
  artifactd or in its protocol.

## Release-blocking caller findings

1. **High — production Node still has a complete Zig Artifact path.** The
   `build.zig.zon` sibling path and the six build imports make a clean Node
   production build depend on the old package. `artifact_runtime`,
   `artifact_integration`, and the preparation path instantiate and execute
   Zig Store/Node/Registry code. This directly violates the requested zero Zig
   Artifact implementation/importer/protocol/fallback gate.
2. **High — no qualified Node-to-artifactd caller handoff is present in these
   files.** The current call sites do not use the v3 Rust protocol, do not
   carry artifactd `LeaseId`/`PreparedArtifactId`, and still hand Sandboxd a
   registry URL plus digest. Removing the dependency before implementing the
   typed client would silently drop pull, graph verification, config policy,
   pin recovery, and prepared-rootfs ownership.
3. **Mandatory — the current synthetic Node tests are coupled to Zig Store and
   Node.** They cannot qualify the Rust daemon boundary. A real daemon test
   must cover FD delivery, peer identity, digest-pinned pull/resolve, lease
   lifetime, prepared FD consumption, restart recovery, and exact binding
   release before the old tests are removed.

The caller cutover therefore remains open. This review does not approve the
artifactd release and does not claim that Node, Sandboxd, or all production
callers have migrated.
