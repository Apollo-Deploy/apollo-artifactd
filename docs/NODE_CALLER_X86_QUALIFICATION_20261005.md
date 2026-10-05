# Node artifactd caller qualification (x86_64)

This is scoped evidence for the Rust artifactd caller cutover. It is not a
release approval for artifactd.

## Host and source

The run used `ssh tihan-apollo` and an owned checkout at
`/home/tihan/artifactd-node-native-new`. The Node executable was built for
`x86_64-linux-gnu` with Zig 0.16.0 and the Rust `artifactd-client-ffi` shared
library. The source hashes copied into that checkout matched the local Node
caller sources at the time of the run:

| input | SHA-256 |
| --- | --- |
| `build.zig` | `e32399b1e627f43ea94e0e1c901449e6a2e91270ddcc506983f10866f8df2e79` |
| `src/main.zig` | `161d4b5979b223033b3292e65eece87656e29ea64855cdc67f20eec612a01b2b` |
| `src/workload/artifactd_client.zig` | `2295fee6eda16e12c198010aa25fbf86ae9d7120e6ff06ad218a42ec19095dc5` |
| `src/workload/artifact_application_core.zig` | `e435db4d4b2f09c8b89321752ae5207baa846106e01487f1320e0d7e47a843f1` |
| `src/workload/artifact_prepare.zig` | `e82f335e511a3132c6ff749844a53ce76b8ae298cc59f3c6e2061e94028ad50f` |
| `src/workload/artifact_prepared_workload.zig` | `1daf069db3928f9b6987487d4ae1a2b83705e26bdb2ac2d16c2fcfef6b1b19f3` |
| `artifactd-client-ffi.so` | `a84db513d38b38e4a4480ab697df90dea1fd7fba6430d3c9824d266e40af2f7b` |

The owned artifactd release binary used by the probe was
`f90c45c6af42df402749b883526f9f557236cd763815b5ea025255dbc2c1eff5`.

## Boundary exercised

An owned private artifactd instance was started with an owned Unix socket and
state directory. An owned OCI archive was imported. The actual Node
`Application` boundary then resolved the image, created its durable binding,
read verified runtime configuration through the Rust FFI, prepared the
selected platform manifest, and completed the restart-equivalent binding
path. The probe emitted:

```text
NODE_ARTIFACT_X86_PREPARE_OK
NODE_ARTIFACT_X86_RESTART_OK
```

The imported index digest was
`sha256:90fd2e16fa9f5f67bf6649a70230969747523bdd7c07f1a528ddde03bdfdd05f`;
the selected manifest was
`sha256:fc536dcbb129e1b4c8a27748839f3aa283d0aa244311b80d9acaadae64f7a481`.

## Handoff boundary and scope

The current Node runtime feeds the MicroSandbox adapter with a desired
workload and a verified generic artifact digest. It has no existing sandboxd
socket/client route. Consequently this run does not claim a Node-to-sandboxd
`ImportPrepared` handoff or an `OPEN_PREPARED` FD handoff. Sandboxd's separate
Rust producer/consumer path requires the generic tuple
`prepared_artifact_id`, `manifest_digest`, `lease_id`, and `architecture`,
with the lease live through consumption; that path is qualified separately by
the sandboxd native handoff fixture. Any future orchestration that connects
Node to sandboxd must be explicitly owned and Rust-protocol based. Adding a
second Zig protocol client would violate the cutover.

This evidence qualifies the Node-to-artifactd preparation path only. It does
not qualify sandbox execution, registry authentication from the Node caller,
fault campaigns, or the artifactd global release gates.
