# apollo-artifactd

`apollo-artifactd` is a standalone Rust service for storing, verifying, transferring, and preparing OCI artifacts on Linux. It provides a local Unix-domain socket API and two binaries:

- `apollo-artifactd` runs the service.
- `apollo-artifactctl` is a command-line client for the versioned API.

The service owns artifact bytes and metadata. It does not run workloads, invoke shells, or depend on another Apollo package. Artifact digests use canonical `sha256:<hex>` identities; registry tags are mutable references, not artifact identities.

## Capabilities

- Stream blobs into a content-addressed store, hashing and checking declared size before atomically publishing content.
- Admit OCI layouts and archives, verify descriptor digest, size, media type, and graph relationships, and resolve platform-specific manifests.
- Pull digest-pinned images from OCI registries over HTTPS and push verified blobs before manifests and optional tags.
- Retain reachable content with pins and leases, and incrementally collect unreferenced owned content.
- Prepare immutable root filesystems by applying verified layers in order, checking DiffIDs, handling whiteouts, and rejecting unsafe archive entries.
- Recover durable operations after restart using a transactional embedded database.
- Pass file descriptors over the local API for imports and reads; enforce peer identity with Linux `SO_PEERCRED`.

Supported OCI platforms include `linux/amd64` and `linux/arm64`. The generic protocol is in [`crates/artifactd-protocol`](crates/artifactd-protocol); a Rust/C client library is in [`crates/artifactd-client-ffi`](crates/artifactd-client-ffi).

## Requirements

- Linux x86_64 or aarch64 for the daemon and local API.
- Rust 1.93 or newer.
- A C toolchain and standard build tools for native compression and TLS dependencies.

The repository includes `Cargo.lock` and the small, license-preserving vendored dependency patches it builds from. It needs no Zig compiler, sibling Apollo checkout, external database, or shell-based import bridge.

## Build

```sh
cargo build --locked --release --bins
```

The outputs are `target/release/apollo-artifactd` and `target/release/apollo-artifactctl`.

## Run locally

Create private directories owned by the dedicated service user, then start the daemon:

```sh
install -d -m 0700 /var/lib/apollo-artifactd /run/apollo-artifactd
apollo-artifactd \
  --store /var/lib/apollo-artifactd \
  --socket /run/apollo-artifactd/artifactd.sock
```

The daemon requires an existing private store and socket parent. It holds an exclusive store lock, and by default accepts requests from its effective UID/GID. An optional policy file can grant configured producer, consumer, and administrator roles to other local identities. See [`deploy/apollo-artifactd.service`](deploy/apollo-artifactd.service) for a systemd example.

Query service capabilities:

```sh
apollo-artifactctl \
  --socket /run/apollo-artifactd/artifactd.sock \
  --action '{"operation":"CAPABILITIES"}'
```

For blob import, the CLI opens the input file and passes its descriptor to the daemon. Filesystem paths are not part of the service request:

```sh
apollo-artifactctl \
  --socket /run/apollo-artifactd/artifactd.sock \
  --input /path/to/blob \
  --action '{"operation":"IMPORT_BLOB","size":123}'
```

The service streams and hashes the input and returns the resulting digest. To enforce a producer-supplied identity, include `"digest":"sha256:<64 lowercase hex>"` in the request; a mismatch fails before publication. Consult `apollo-artifactctl --help` and the protocol crate for the complete request and response contract.

## Registry transfers

The caller supplies the registry and repository. Pulls use digest-pinned references, for example `registry.example.com/team/image@sha256:<digest>`. The service verifies remote content and OCI relationships before admitting it. Push uploads verified config and layer blobs first, then manifests or indexes, and can update a tag afterward.

There is no built-in registry destination. Credentials are provided through a protected file descriptor, not command-line arguments, logs, or persistent artifact state. Anonymous registry access needs no credential provider. Network and credential-provider policy is enforced by the daemon.

## Ownership and lifecycle

Pins and leases protect an artifact and its reachable OCI graph from collection. Keep the returned generic IDs in the caller's durable lifecycle state and release them when the artifact is no longer needed. `OPEN_BLOB` and `OPEN_PREPARED` return descriptors only when the caller has an applicable lease. Garbage collection is bounded and uses persisted progress so it can resume after interruption.

Prepared artifacts are identified by the source manifest digest, canonical platform, and preparation format version. The service validates layers before publishing a prepared rootfs and rejects traversal, escaping links, hard links, devices, malformed archives, and entries exceeding configured limits.

## Configuration and operations

The daemon exposes `STATUS`, `DOCTOR`, `RECONCILE`, and `CAPABILITIES` through the local API. Limits and access policy are supplied at daemon startup. Registry URLs and credentials are caller-provided; no Apollo control-plane or secrets service is required.

The service is designed to run unprivileged under a dedicated OS account with exclusive ownership of its private store. Linux descriptor-relative filesystem operations anchor storage access to the configured directories. Review the systemd example and host-specific deployment policy before enabling cross-UID access.

## Development

```sh
cargo check --locked --workspace --all-targets
```

Fuzz targets live in the separate `fuzz/` Cargo workspace and require `cargo-fuzz`. The service and protocol are modular Rust crates; vendored dependency changes and their provenance are documented beside the vendored sources.

## License

The Apollo Artifactd workspace is licensed under the MIT License. Vendored dependencies retain their upstream licenses and notices; see `vendor/` and `licenses/`.
