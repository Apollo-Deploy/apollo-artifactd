# Public API churn qualification

This is a bounded smoke qualification of `examples/api_churn.rs`, not the required
100,000 blob / 20,000 OCI campaign. The runner rejects counts above those limits,
uses fresh daemon operation tokens, replays one completed import, verifies returned
blob and prepared FDs, restarts while protected OCI content is live, performs a
post-restart `STATUS`, then stops the daemon before opening the state database for
the final inventory.

The qualified source hashes are:

```text
c6f18c76a96b9262e7f62f4b0ed857fee5e654c590ecef6a037580cf0ea11014  examples/api_churn.rs
0e17301edf1be665d24e8bac78ed479efb897171af1a33134e077bf3a22f2edd  examples/api_churn/cycles.rs
b964f148fbfe9d029e0a9521cbfec0301d68f87d948bbab5577b1e5dc596a885  examples/api_churn/metrics.rs
e8b91a0f46cd3fbe6b435764fe61805da0d12a10e860f9b2b9c68b87a9c86a8b  examples/api_churn/oci.rs
```

## ARM64: `apollo-node-01`

Commands:

```text
ssh apollo-node-01 'cd ~/artifactd-qualification-20261004 && ~/.cargo/bin/cargo check --example api_churn'
ssh apollo-node-01 'cd ~/artifactd-qualification-20261004 && d=$(mktemp -d /tmp/artifactd-api-churn.XXXXXX); mkdir -m700 "$d/store" "$d/run"; ~/.cargo/bin/cargo run --quiet --example api_churn -- target/debug/apollo-artifactd "$d/store" "$d/run/artifactd.sock" 1 1; rm -rf "$d"'
```

`cargo check` passed. The smoke passed with blob PID `330362`, OCI PID `330363`,
and final restart PID `330364`:

```text
blob_operations=1 oci_lifecycles=1 bytes={blob:10,oci_archives:7680} elapsed_us=138333 throughput_bytes_per_sec=55590.09
operation_latency={import_blob:6441us,import_archive:18616us,prepare:9951us,gc:8191us}
status={blobs:0,bytes:0,production_qualified:false,schema:3}
final_inventory={prepared_entries:0,store_entries:5,blobs:0,edges:0,imports:0,leases:0,operations:17,pins:0,prepared:0,roots:0}
```

## x86_64: `tihan-apollo`

Commands:

```text
ssh tihan-apollo 'cd ~/artifactd-qualification-20261004 && /usr/bin/cargo check --example api_churn'
ssh tihan-apollo 'cd ~/artifactd-qualification-20261004 && d=$(mktemp -d /tmp/artifactd-api-churn.XXXXXX); mkdir -m700 "$d/store" "$d/run"; /usr/bin/cargo run --quiet --example api_churn -- target/debug/apollo-artifactd "$d/store" "$d/run/artifactd.sock" 1 1; rm -rf "$d"'
```

`cargo check` passed. The smoke passed with blob PID `2287123`, OCI PID
`2287128`, and final restart PID `2287132`:

```text
blob_operations=1 oci_lifecycles=1 bytes={blob:10,oci_archives:7680} elapsed_us=203350 throughput_bytes_per_sec=37816.39
operation_latency={import_blob:7110us,import_archive:21878us,prepare:13092us,gc:11045us}
status={blobs:0,bytes:0,production_qualified:false,schema:3}
final_inventory={prepared_entries:0,store_entries:5,blobs:0,edges:0,imports:0,leases:0,operations:17,pins:0,prepared:0,roots:0}
```

Resource snapshots included RSS, HWM, thread count, FD count, CPU ticks, and
state size. Both runs used architecture-specific OCI metadata (`arm64` on ARM64
and `amd64` on x86_64), and compared the resolved manifest digest with the
independently generated manifest digest.

The source was subsequently refactored into `cycles.rs` and strengthened with
recursive final filesystem emptiness checks and cumulative CPU segment reporting.
The 1+1 outputs above predate that refactor; a fresh 10+5 native run is required
before treating the current hashes as requalified evidence.
