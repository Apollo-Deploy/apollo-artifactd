# Independent implementation red-team review — 2026-10-05

An independent reviewer applied the `security-red-team` skill to the current
`apollo-artifactd` source and private local fixtures. This was a source-assisted,
read-only review; it did not probe deployed systems, production stores, or
unrelated services, and it made no source changes.

The review traced CAS digest/size verification and atomic publication; OCI
descriptor, graph, platform and DiffID validation; archive traversal, symlink,
whiteout, hardlink and device handling; Unix `SO_PEERCRED`, packet/FD parsing
and policy authorization; registry credentials, TLS, redirects, digest-pinned
pulls and push ordering; pin/lease graph reachability, GC ownership and epochs;
and crash recovery and journal replay.

The reviewer reported no confirmed unauthorized read, write, execution,
filesystem escape, credential disclosure, descriptor substitution, GC
ownership violation, or protocol-authentication bypass. The focused local
checks passed: 14 CAS tests, 9 OCI/preparation tests, 8 recovery tests, 8
preparation-extension tests, 2 GC tests and 2 registry tests. `git diff
--check` passed.

This review does **not** grant `RED_TEAM_RELEASE_APPROVED`. The requested native
fuzz campaigns are incomplete; native x86 qualification, full caller and
Zig removal, structural database-corruption testing, bounded global-GC
qualification and the remaining production gates are incomplete. Repeat the
independent review against the final qualified cutover before claiming release
approval.

## GC reachability follow-up

After the above review, an independent read-only re-review of the updated GC
reachability slice closed two HIGH findings: a modified persisted edge set can
no longer omit or substitute reachable OCI descriptors, and deleting root
classification records cannot make a valid OCI graph look like raw content.
The collector rebuilds the expected graph from digest-verified metadata,
checks exact persisted-edge equality and required root classifications, and
uses bounded ownership/size checks for layer files instead of rehashing them
on each GC pass. The new corrupted-live-layer regression confirms later reads
still reject the damaged bytes while GC preserves the reachable path.

The reviewer reran `gc_incremental`, `oci`, and `oci_gc`: 3, 9, and 1 tests
passed. It found no remaining HIGH or mandatory MEDIUM within the documented
attacker boundary and approved this slice as `RED_TEAM_RELEASE_APPROVED`. The
same-UID pathname replacement race in `owned_remove_blob` remains structurally
present and is excluded only under the dedicated service-UID threat boundary.
This scoped approval does not supersede the release gate in
`docs/QUALIFICATION.md`; native x86 requalification, full churn/fuzz and caller
cutover gates remain open.
