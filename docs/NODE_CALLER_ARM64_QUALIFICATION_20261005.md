# Node artifactd caller qualification — ARM64

Host: `apollo-node-01`, Linux `aarch64`, Zig `0.16.0`.

The isolated public caller probe used the Node `Application` boundary and a
private artifactd daemon/store/socket. It imported a valid OCI archive pinned
as `node-caller-pin`, then exercised:

- `Application.start` recovery against an existing binding directory;
- index resolution with native `linux/arm64/v8` selection;
- verified runtime-config decoding;
- selected manifest preparation;
- temporary lease creation and release;
- prepared-artifact FD open and close;
- prepared binding release and a second process-equivalent restart run.

The probe emitted `NODE_ARTIFACT_PREPARE_OK` and
`NODE_ARTIFACT_PREPARED_FD_OK`. The temporary probe executable, private store,
private socket, and binding directory were removed after the run.

The final caller build was:

```text
zig build -Dtarget=aarch64-linux-gnu -Doptimize=ReleaseSafe \
  -Dartifactd-client-lib=/home/apollo-admin/artifactd-node-native-new/apollo-artifactd/target/debug/libartifactd_client_ffi.so
```

Observed final binary:

```text
apollo-node sha256:591691e279bf78bc5a242a66508df2e8b61404d09400566d0cb440fc206fced1c
```

Current source hashes:

```text
artifactd_client.zig             2295fee6eda16e12c198010aa25fbf86ae9d7120e6ff06ad218a42ec19095dc5
artifact_application.zig         0419b87342c7d96506bcb5d42097c44a7e4851e9adff02376283f17aa1b594a0
artifact_application_core.zig    e435db4d4b2f09c8b89321752ae5207baa846106e01487f1320e0d7e47a843f1
artifact_application_helpers.zig 9fb94fc609a773816161688b56c9020327dba0b9d52efc37cdeeb9011491ffa5
artifact_prepared_workload.zig   1daf069db3928f9b6987487d4ae1a2b83705e26bdb2ac2d16c2fcfef6b1b19f3
artifact_prepare.zig             e82f335e511a3132c6ff749844a53ce76b8ae298cc59c3f6e2061e94028ad50f
artifact_identity.zig            3db897288dd88dcb9a4b756475a37dda08649cde45e8bfdaa491abf433fa6259
```

This evidence covers the Node artifactd caller and ARM64 only. It does not
qualify MicroSandbox execution, a current x86_64 caller build, registry
authentication through the Node path, full restart/fault campaigns, or the
remaining artifactd production gates.
