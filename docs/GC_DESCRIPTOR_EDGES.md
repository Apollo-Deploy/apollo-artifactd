# GC preserves surviving descriptor relationships

A manifest's descriptor relationships are immutable even when its unpinned
children are collected. Previously, deleting one child rewrote every parent's
persisted edge list in one transaction. Restoring the exact config bytes then
left a surviving manifest impossible to pin: its bytes still described the
config, but the persisted relationship had disappeared.

GC now removes only the collected object's outgoing edges and its own root,
blob, and deletion-intent records. Surviving parents keep their descriptor
edges. A missing child still makes pinning and resolution fail; importing the
exact leaf bytes restores the admitted graph, including after restart. A
deleted OCI manifest/index still requires OCI admission to restore its own
outgoing relationships. Raw blob import does not bypass OCI admission.

Dangling edges are deliberate incomplete-cache metadata. They do not authorize
deletion or exposure: protected-root graph verification must succeed, and a
protected parent protects its reachable children. An edge row belongs to its
parent and is removed when that parent is collected. GC never scans all edge
rows to rewrite other parents during one object's deletion.

## Regression evidence

`tests/oci_gc.rs` exercises production Store calls with independently hashed
zero-layer OCI config/manifest bytes. Digest ordering selects the config first
under `GC(1)`. The test checks missing-child refusal, exact reimport, restart,
successful pin of the unchanged manifest, and preservation under subsequent GC.

The test-audit authoring gate is satisfied:

- Contract: restoring exact descriptor bytes restores a surviving admitted
  manifest's usability, while incomplete graphs remain unavailable.
- Pre-fix regression: deleting a leaf stripped the parent's descriptor edge, so the
  post-reimport pin fails with `persisted OCI reachability mismatch`.
- Existing tests protected pinned graphs and cursor progress; none restored a
  collected child of a surviving unpinned parent.
- The test uses public Store operations with no fault flag or test-only export.

Actual pre-fix failures are retained in
`oci-gc-edge-pre-fix-{arm64,x86_64}.txt`; both reach the intended post-restart pin
failure. The original macOS reproduction is `oci-gc-edge-pre-fix.txt`.
Post-fix native regression, normal workspace suites, and clippy results are
recorded in `oci-gc-edge-final-{arm64,x86_64}.txt`.
Both `oci-gc-edge-snapshot-{arm64,x86_64}.json` snapshots match 71 current
source/dependency/deployment files. [Independent review](RED_TEAM_GC_EDGES.md)
accepts this specific repair and retains the required release gates.

## Remaining required GC work

This repair removes the global edge rewrite; it does not qualify globally
bounded GC. Live-root verification and per-candidate pin/lease/prepared-root
reachability still scan all handles. Those scans require persisted incremental
progress, protection against intervening reference/graph mutations, and native
crash/race/resource qualification before release approval.
