# apollo-artifactd

**APOLLO_ARTIFACTD_PRODUCTION_PARTIAL.** This is an independently buildable Rust artifact service and its own protocol crate. It is not qualified for the requested complete production cutover. Registry transfers are unavailable, existing Apollo callers still use the old implementation, and independent reviewers block release. See [qualification](docs/QUALIFICATION.md) and [migration map](docs/MIGRATION.md).

The package implements streaming SHA-256 CAS, transactional SQLite intents, OCI indexes/manifests/configs/layers, local archive admission, graph pins and leases, ownership checked collection, verified layer preparation and descriptor passing. It never runs customer workloads or invokes shells. It has no Zig code and no dependencies on Apollo sibling packages.

## Build and run

Use native Linux x86_64 or aarch64, Rust 1.93 or newer, a C compiler/linker and standard build utilities for bundled SQLite and zstd. No external database, Zig compiler or sibling checkout is required. Rust dependency versions are recorded in Cargo.lock. macOS supports library tests, but the binaries require Linux SO_PEERCRED and SOCK_SEQPACKET.

```sh
cargo build --locked --release --bins
cargo test --locked --workspace --all-targets
mkdir -m 700 /absolute/private/artifact-store /absolute/private/artifact-runtime
target/release/apollo-artifactd \
  --store /absolute/private/artifact-store \
  --socket /absolute/private/artifact-runtime/artifactd.sock
```

Run as an unprivileged dedicated user. The root and socket parent must already exist, be owned by that user and be private. An exclusive store lock prevents two daemons owning the same state. The included systemd unit is a qualification example, not an installed service. Its network restriction intentionally matches this local-only build.

```sh
target/release/apollo-artifactctl \
  --socket /absolute/private/artifact-runtime/artifactd.sock \
  --action '{"operation":"CAPABILITIES"}'
target/release/apollo-artifactctl \
  --socket /absolute/private/artifact-runtime/artifactd.sock \
  --operation-id import-example \
  --input /absolute/path/to/blob \
  --action '{"operation":"IMPORT_BLOB","digest":"sha256:<64 lowercase hex>","size":123}'
```

The CLI opens local input and sends its FD. Paths do not cross the daemon API. For OPEN_BLOB/OPEN_PREPARED, use `artifactd_protocol::client::call` on Linux: it returns an owned descriptor that the caller retains. The CLI reports and closes returned descriptors on exit. The client package needs no daemon implementation or Apollo business types.

## Protocol and ownership

One version-1 JSON packet per Unix SOCK_SEQPACKET connection, at most 64 KiB and one SCM_RIGHTS FD. Digests are canonical lowercase `sha256:` identities. Import descriptors must be bounded regular files. OPEN requires a live lease. Replayed reads recheck current integrity and liveness. Mutations persist intent and completion; history retains approximately 4096 completed/failed records, so long-term replay guarantees do not meet the requested release gate.

Both peers authenticate SO_PEERCRED against the current effective UID. This implementation assumes cooperating processes under the dedicated service UID. A read-only FD and filesystem mode do not prevent that UID from changing inode permissions. Hostile same-UID isolation and cross-UID consumers are not qualified. SQLite validates an FD before reopening its pathname; that remaining same-owner TOCTOU window is a high-severity release finding in [red-team round 3](docs/RED_TEAM_ROUND_3.md).

Default bounds: 4 GiB blob, 16 GiB store, 4 MiB OCI metadata, 128 graph descriptors, 100,000 rootfs entries, depth 128, and 4096 deletion records per GC call. The daemon processes requests serially; bounded storage does not guarantee responsive concurrent clients. GC validates live graph integrity but scans reference reachability globally.

Prepared identity includes the manifest digest, canonical platform and preparation version. Output hashes include content, paths, relative symlinks and execute bits; ownership is the dedicated daemon user and timestamps are not identity. Absolute links, hardlinks, devices, special entries, GNU/PAX extension headers and unsupported descriptor features are rejected. These restrictions are explicitly incomplete portability versus the requested OCI/rootfs behavior.

## Qualification tooling

`examples/churn.rs` performs 100,000 blob import/pin/unpin/GC cycles followed by 20,000 OCI cycles. Its OCI fixture has no layers; it does not represent all image workloads or daemon operation-journal churn. `examples/throughput.rs` measures streaming import and verified preparation using a 256 MiB uncompressed layer. `scripts/measure-idle.py` measures an isolated daemon. Each needs a new administrator-provisioned private store; it never touches existing services.

The separate fuzz workspace uses cargo-fuzz/libFuzzer for protocol/OCI parsing, archive admission/preparation and corrupt recovery records. Short sanitizer runs are smoke evidence, not completed fuzz qualification. Registry responses have no fuzz target because registry support is absent.

Dependency evidence, CycloneDX SBOM, native logs and review reports are in `docs/`. The existing Zig artifact package and importer remain because this replacement has not passed its cutover gate. No production deployment was performed.
