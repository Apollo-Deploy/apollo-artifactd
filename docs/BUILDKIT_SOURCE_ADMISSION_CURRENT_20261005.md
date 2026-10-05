# Private source and Build admission qualification

The production control-plane binary in the isolated x86 fixture completed migrations (262), service-admission and build-start wrapper privilege revocation checks, authenticated 17,825,894-byte SourceBundle upload, cancellation/replay/source-lease revocation, and Build admission/replay. The retained run is `buildkit-source-admission-x86_64-20261005.log`. This is admission evidence, not completed BuildKit solve, artifact publication, sandbox execution or release qualification.

The fixture corrected its cross-project check to the actual complete build-read route, using a separately seeded project and requiring HTTP 403. Authoritative and remote script hash reported by the fixture owner: `43551a3bef4f3f7e4b74de09f5fa10518601f90f3925812ae1a00918b0e170b3`. The isolated HTTPS certificate uses server EKU/SAN and CA:FALSE. Fixture processes were stopped after the run; no production deployment occurred.

The missing isolated sandbox-server binary and campaign/compilation artifacts must be built/staged before the authenticated end-to-end execution fixture can run. The source/admission pass does not close that integration gate.
