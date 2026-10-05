# Registry response fuzz qualification — 2026-10-05

The `registry_responses` cargo-fuzz target invokes the same vendored `oci-client` manifest and OCI error-envelope decoders used by the registry client. Successfully decoded responses must retain semantic JSON and media type through serialization/reparsing. Inputs are limited to 64 KiB. Five synthetic seeds include manifest, index, authentication error, unknown error and nested error detail. No credentials or customer data enter the corpus.

`cargo check --manifest-path fuzz/Cargo.toml --bin registry_responses` passed. The fuzz lockfile adds only the existing `oci-client` package to the fuzz package's direct dependency list; no new package versions were introduced.

An Apple Silicon macOS smoke campaign passed:

```
cargo +nightly fuzz run registry_responses -- -max_total_time=30 -max_len=65536 -rss_limit_mb=1024
```

Terminal exit: zero; 101,225 executions in 31 seconds. Full build/run output: `fuzz-registry-responses-smoke-20261005.log`. Independent source review: `RED_TEAM_REGISTRY_FUZZ.md`. This is decoder smoke qualification, not native Linux/long fuzz qualification or validation of HTTP/TLS/authentication/redirect/streaming policy. Those required gates remain open. Digest parsing and OCI configuration decoding are already exercised by `protocol_oci`; dedicated target names are not required to establish that coverage.

Source SHA-256: `3d520f791719262155946ae44a44088284a47e2dd9d663e6215ff251f9e34ec8`. Fuzz manifest SHA-256: `400d69556678b4b94caa988fbfb9d718d79f474cc308da2681213a06f6bebbfd`. Fuzz lock SHA-256: `8070b6a85b4da86c020c1e8fcb10abbd63513ebef99076d1727fc9a53c92f255`. Final decoder coverage was 2,277 edges / 5,852 features; sampled fuzzer RSS was 299 MiB. These are fuzzer process observations, not daemon RSS.
