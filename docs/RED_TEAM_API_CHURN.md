# Public API churn runner scoped red-team review

This is an independent static review of the frozen public daemon runner in
`examples/api_churn.rs` and its included cycle, OCI, and metrics modules. It
does not report execution or release approval.

## Reviewed source

| File | SHA-256 |
|---|---|
| `examples/api_churn.rs` | `c6f18c76a96b9262e7f62f4b0ed857fee5e654c590ecef6a037580cf0ea11014` |
| `examples/api_churn/cycles.rs` | `0e17301edf1be665d24e8bac78ed479efb897171af1a33134e077bf3a22f2edd` |
| `examples/api_churn/metrics.rs` | `b964f148fbfe9d029e0a9521cbfec0301d68f87d948bbab5577b1e5dc596a885` |
| `examples/api_churn/oci.rs` | `e8b91a0f46cd3fbe6b435764fe61805da0d12a10e860f9b2b9c68b87a9c86a8b` |

## What the runner does prove

The requested default counts are enforced (`100,000` blob operations and
`20,000` OCI lifecycles). Blob cycles import a digest and size, read back the
returned blob FD, create and release a lease, pin and unpin, and invoke GC.
OCI cycles use a nonempty one-layer OCI archive with a matching config DiffID,
resolve the selected platform/manifest, prepare it, read the prepared payload
through an FD, release the lease, unpin, and invoke GC. One completed blob
import is replayed, and one prepared artifact is opened after a SIGKILL-style
restart. Latency samples are bounded vectors with p50/p95/p99 output; RSS/HWM,
threads, FDs, state size, and CPU ticks are sampled.

## Mandatory qualification gaps

1. The restart checks occur after a completed import/preparation. There is no
   interrupted import, preparation, pin, lease, or GC intent during the churn
   run, and there is no repeated crash/restart matrix. A crash-safe result
   therefore remains unqualified.
2. Only the first blob import is replayed. OCI import, resolve, prepare,
   lease, pin, unpin, and GC operation tokens are not individually replayed
   after ambiguous responses. The runner does not deliberately drop a response
   before replaying the same token.
3. The recursive inventory and zero-row checks now cover the `blobs`, `temp`,
   and `prepared` trees, with a shared 10,000-entry traversal bound. The prior
   orphan-directory finding is closed for those trees. The check still does
   not remove the caller-provided store, and it does not independently inspect
   arbitrary foreign files outside the owned state directories.
4. CPU accounting now records baseline, pre-first-restart, post-first-restart,
   pre-final-restart, post-final-restart, and final readings. This is materially
   stronger. With both cycle counts set to zero, however, there is no workload
   CPU segment and the corresponding fields correctly fall back to zero; a
   zero-cycle run must not be presented as performance evidence.
5. There is no registry pull/push, credential-FD, multi-platform, or
   cross-peer API lifecycle in this runner. Its OCI fixture covers one platform
   and one layer shape only.
6. Failure cleanup is mostly RAII based, but there is no injected failure at
   each lifecycle boundary proving that child processes, returned FDs, leases,
   pins, temporary files, and the store are reconciled after every error.

The runner is useful bounded public-API evidence, but these gaps prevent it
from being treated as proof of the complete requested churn, crash-recovery,
registry, or performance gates. No global release approval is implied.
