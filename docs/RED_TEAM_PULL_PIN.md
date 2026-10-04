# Red-team review: registry pull pinning and local cache

Date: 2026-10-05  
Scope: `src/registry/pull.rs`, pull API dispatch, pinned pulls, and the local
cache path. This is a scoped implementation review; it is not a release gate.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`. I found no confirmed pin omission,
credential persistence, graph substitution, or GC-protection defect in the
current pull implementation. The complete registry/fault/recovery and
production release gates remain outside this conclusion.

## Reviewed behavior

- `pull_pinned` validates the protocol `PinId` before the local-cache or
  network path. Both paths end in `admit_oci_with_pin`, which atomically writes
  the verified graph edges, roots, and optional pin. A local graph hit therefore
  cannot silently omit the requested pin.
- Pull references must contain a valid SHA-256 digest. Manifests and indexes
  are fetched by digest, each descriptor is size/media-type checked, and the
  completed graph is independently revalidated by artifactd before admission.
  Multi-platform index traversal retains all reachable image/config/layer edges
  while selecting the requested platform.
- A cache hit does not perform a registry request, so its credential authority
  is not used for remote authentication. The API still reads the private
  credential FD before dispatch; no credential material enters the local cache,
  graph state, admission facts, or journals. This is the expected behavior for
  an already verified digest-pinned local result.
- If a network pull or pin conflict fails after individual blobs were streamed
  and verified, those blobs remain unrooted CAS content until GC. They are not
  admitted as an OCI graph or exposed as a successful pin; this is bounded
  recoverable cache state, but the broader orphan-churn and crash campaign is
  still required.
- Retry/replay uses the same digest-pinned reference and the common admission
  path. Same-pin/same-digest replay is idempotent; a different digest under an
  existing pin is rejected.

## Evidence boundary and remaining work

The public API test now exercises the cache path as a separate operation: it
admits the same digest-pinned graph under a distinct `PinId`, unpins the
original network pin, runs GC, verifies that the graph is still live, and only
then resolves and prepares it. This demonstrates the requested cache pin has a
real GC effect rather than merely being echoed in the response. The available
ARM64 and x86_64 registry evidence records the authenticated graph/retry,
credential/journal, and registry SIGKILL cases passing
(`docs/archive-paths-fault-registry-arm64.txt`,
`docs/archive-paths-fault-registry-x86_64.txt`). Those fault logs predate the latest
cache-pin test-support changes and are scoped historical evidence, not blanket
qualification of the current source.

The controlled pre-fix network run reached the HTTPS registry but failed the
public assertion because the pin receipt was omitted. After two discarded
connection-refused readiness attempts, the separate cache-only pre-fix control
reached its semantic assertion and failed because `registry-api-cache` was
also omitted (`docs/pull-pin-cache-pre-fix-arm64.txt`). This independently
confirms the old cache omission. The repaired behavior is supported by the
strengthened current public API test, and the final ARM64/x86_64 reruns passed the authenticated registry API test,
normal workspace suite and clippy checks. Exact snapshots match the current
source. Both production release builds passed; x86 test support was subsequently
strengthened and rerun without changing production code. These results do not qualify
every registry response, redirect/authentication, orphan-churn, power-loss,
global GC, caller-cutover, or old-Zig removal gate.
