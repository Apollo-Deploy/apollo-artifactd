# apollo-artifactd

**APOLLO_ARTIFACTD_PRODUCTION_PARTIAL.** This is an independently buildable Rust artifact service and its own protocol crate. It is not qualified for the requested complete production cutover. Authenticated HTTPS registry transfers are implemented and have native integration evidence; existing Apollo callers still use the old implementation, and independent reviewers block release. See [qualification](docs/QUALIFICATION.md) and [migration map](docs/MIGRATION.md).

The package implements streaming SHA-256 CAS, transactional redb intents, OCI indexes/manifests/configs/layers, local archive admission, graph pins and leases, ownership checked collection, verified layer preparation and descriptor passing. It never runs customer workloads or invokes shells. It has no Zig code and no dependencies on Apollo sibling packages.

## Build and run

Use native Linux x86_64 or aarch64, Rust 1.93 or newer, a C compiler/linker and standard build utilities for zstd and the TLS cryptography backend. No external database, Zig compiler or sibling checkout is required. Rust dependency versions are recorded in Cargo.lock. macOS supports library tests, but the binaries require Linux SO_PEERCRED and SOCK_SEQPACKET.

```sh
cargo build --locked --release --bins
cargo test --locked --workspace --all-targets
mkdir -m 700 /absolute/private/artifact-store /absolute/private/artifact-runtime
target/release/apollo-artifactd \
  --store /absolute/private/artifact-store \
  --socket /absolute/private/artifact-runtime/artifactd.sock
```

Run as an unprivileged dedicated user. The root and socket parent must already exist, be owned by that user and be private. An exclusive store lock prevents two daemons owning the same state. The included systemd unit is a qualification example, not an installed service. Its address-family policy permits the local Unix API and outbound registry HTTPS connections.

```sh
target/release/apollo-artifactctl \
  --socket /absolute/private/artifact-runtime/artifactd.sock \
  --action '{"operation":"CAPABILITIES"}'
target/release/apollo-artifactctl \
  --socket /absolute/private/artifact-runtime/artifactd.sock \
  --input /absolute/path/to/blob \
  --action '{"operation":"IMPORT_BLOB","digest":"sha256:<64 lowercase hex>","size":123}'
```

For a trusted local producer, omit `digest` from `IMPORT_BLOB`; artifactd calculates and returns SHA-256 while streaming, still enforcing the declared size and quotas. The resolved digest is committed to the import intent before publication.

`IMPORT_OCI`, `IMPORT_OCI_ARCHIVE` and `PULL` accept an optional generic `pin` string. When supplied, verified graph admission and the pin commit in one transaction, and the receipt returns `pin_id`. A conflicting existing pin fails without replacing it. Producers must retain the pin until their artifact lifecycle authorizes `UNPIN`; an import without a pin remains eligible for GC.

The CLI opens local input and sends its FD. Paths do not cross the daemon API. For OPEN_BLOB/OPEN_PREPARED, use `artifactd_protocol::client::call` on Linux: it returns an owned descriptor that the caller retains. The CLI reports and closes returned descriptors on exit. The client package needs no daemon implementation or Apollo business types.

## Registry transfers

The caller supplies an OCI repository reference. PUSH uploads verified CAS config/layers, then manifests/indexes, then an optional tag. PULL requires a digest-pinned reference such as `registry.example.com/team/image@sha256:<64 lowercase hex>` and verifies remote content before OCI admission. No production registry destination is hardcoded.

PUSH/PULL accept an optional protected credential-provider FD. The CLI opens it with `--input`; credentials themselves must never appear in command arguments. The JSON provider supports `registry` (exact host:port), either `username`/`password` or `token`, optional `ca_pem`, and optional `auth_authorities` (exact trusted bearer-realm host:port values). Use a regular file owned by the service UID, one hard link, no group/other permissions, and at most 64 KiB. Provider content is not persisted in Artifact state. Anonymous access uses no FD. Private provider files must be provisioned outside the store and socket runtime directory.

## Protocol and ownership

One version-2 JSON packet per Unix SOCK_SEQPACKET connection, at most 64 KiB and one SCM_RIGHTS FD. Digests are canonical lowercase `sha256:` identities. Import descriptors must be bounded regular files. OPEN requires a live lease. Replayed reads recheck current integrity and liveness.

Packet delivery waits are bounded to 30 seconds. The client allows up to 600 seconds for an operation response, so verified streaming transfers can complete. A response timeout leaves the operation outcome unknown; retry the same allocated token rather than automatically creating a new mutation.

Mutations use daemon-issued `OperationId` tokens from `OPERATION_ALLOCATE`; the CLI allocates automatically unless `--operation-id` supplies a token for a retry. The durable journal retains up to 4096 reservations/results. Under allocation pressure, it retires the oldest terminal results and unused reservations in batches, advancing a durable retirement floor. A retired token returns `operation expired` and never repeats an effect. Within the retained window, both success and failure replay their original result; a different request with the same token is rejected.

Startup audits the bounded journal before accepting clients. Interrupted intents become terminal failures reporting an uncertain outcome: inspect the artifact/reference or remote registry before attempting a new operation. The daemon does not repeat unknown effects after restart. `DOCTOR` is journaled maintenance: it performs redb page integrity checking and logical graph verification. Repair or check failure quarantines the state and stops the daemon; same-token replay returns a historical inspection receipt. See [Doctor integrity policy](docs/DOCTOR_INTEGRITY.md). Existing nonempty version-1 journals are preserved and rejected pending a qualified migration; no protocol fallback exists. See [operation journal qualification](docs/OPERATION_JOURNAL.md) for native daemon churn and crash evidence and remaining release gates.

The daemon admits peers matching its effective UID. The protocol client authenticates the server using SO_PEERCRED before sending a request or FD. `artifactd_protocol::client::Client::new(socket, server_uid)` and CLI `--server-uid` select the trusted service UID; convenience functions and the CLI default trust the caller UID. This client setting does not grant daemon access. See [client identity qualification](docs/CLIENT_IDENTITY.md). The daemon currently assumes cooperating processes under the dedicated service UID. A read-only FD and filesystem mode do not prevent that UID from changing inode permissions. Hostile same-UID isolation and cross-UID consumers are not qualified. redb uses the capability-opened private FD directly. Legacy state.sqlite stores are rejected without modification; state migration and corruption qualification remain open.

Default bounds: 4 GiB blob, 16 GiB store, 4 MiB OCI metadata, 128 graph descriptors, 100,000 rootfs entries, depth 128, and 4096 deletion records per GC call. The daemon processes requests serially; bounded storage does not guarantee responsive concurrent clients. GC verifies at most 64 root records per mark call using durable cursors and reference epochs; deletion waits for a complete unchanged protection snapshot. Per-root graph reads remain bounded by graph and byte limits. See [incremental GC evidence](docs/GC_INCREMENTAL.md).

Prepared identity includes the manifest digest, canonical platform and preparation version. Output hashes include content, paths, relative symlinks and execute bits; ownership is the dedicated daemon user and timestamps are not identity. Absolute links, hardlinks, devices, special entries, GNU/PAX extension headers and unsupported descriptor features are rejected. These restrictions are explicitly incomplete portability versus the requested OCI/rootfs behavior.

## Qualification tooling

`examples/churn.rs` performs 100,000 blob import/pin/unpin/GC cycles followed by 20,000 OCI cycles. Its OCI fixture has no layers; it does not represent all image workloads or daemon operation-journal churn. `examples/throughput.rs` measures streaming import and verified preparation using a 256 MiB uncompressed layer. `scripts/measure-idle.py` measures an isolated daemon. Each needs a new administrator-provisioned private store; it never touches existing services.

The separate fuzz workspace uses cargo-fuzz/libFuzzer for protocol/OCI parsing, archive admission/preparation and corrupt recovery records. Short sanitizer runs are smoke evidence, not completed fuzz qualification. Registry response fuzz coverage remains missing.

Dependency evidence, CycloneDX SBOM, native logs and review reports are in `docs/`. The existing Zig artifact package and importer remain because this replacement has not passed its cutover gate. No production deployment was performed.
