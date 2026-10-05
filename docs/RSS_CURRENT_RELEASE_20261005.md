# Current-release ARM memory diagnostics — 2026-10-05

The bounded-memory gate remains open. These isolated diagnostics use the
current coherent release daemon, SHA-256
`02cacdbc4a936c9f0238feedcba522df72ab2b7746210ec32166d7bbe4a282ab`,
on `ssh apollo-node-01-internet`. All stores and sockets are private to each
diagnostic under `/home/apollo-admin/artifactd-rss-profile-20261005-arm64`.
The active 100,000-blob/20,000-OCI qualification runs were not altered.

The 172 local and ARM Cargo/source/test/example/vendor input files still
matched at capture. The SHA-256 of their sorted JSON path/hash map is
`fccde001984e512bf21c7fe76a14a421137e37c6b5b732e754c65312ef91c265`;
the lockfile SHA-256 is
`1713a22a5408df6e03dc3f48b888550cb0306d00131f2c8e6ee4e7578996b324`.
This map serialization differs from the aggregate serialization in the
[native snapshot](CURRENT_WORKSPACE_NATIVE_20261005.md).

## Completed smaller request mix

Each cycle imported distinct small bytes through a private temporary-file
FD, checked the returned SHA-256 identity, pinned/unpinned the blob, and
called GC. Mutation tokens came from the daemon's public allocation API.
This mix omits lease creation, blob FD reads, and lease release present in
the actual qualification runner. It therefore cannot attribute that
runner's memory growth.

| Private diagnostic | Cycles | Initial/final RSS (KiB) | Final anonymous RSS (KiB) | Final state bytes |
| --- | ---: | ---: | ---: | ---: |
| `direct-current-default-10000` | 10,000 | 5,336 / 10,856 | 5,896 | 2,433,024 |
| `direct-current-trim-10000` | 10,000 | 5,336 / 10,736 | 5,776 | 2,433,024 |
| `massif-current-10000` | 10,000 | 33,308 / 41,504 | 34,664 | 2,433,024 |

The trim diagnostic set `MALLOC_TRIM_THRESHOLD_=0` only for its own daemon.
Its small difference does not establish an allocator fix. Both direct
diagnostics continued growing after their database-size plateau. All three
workloads finished with `PROFILE_WORKLOAD_PASS`; their private daemon was
then sent SIGTERM and exited with signal status `-15`. SIGTERM uses the
default termination action, so Rust destructors did not run.

Privately extracted Valgrind 3.22.0/Massif wrote 162 snapshots for the
instrumented diagnostic. It reported 5,617,153 bytes of guest heap at peak
and 4,841,630 bytes in the final signal-time snapshot. Guest live bytes rose
during the run; the release ELF is stripped and the retained stacks are
address-only. Neither these totals nor the database plateau identify an
unbounded owner. Profiler RSS includes substantial overhead and is not a
service performance measurement.

The raw 167 MB Massif file remains on ARM at
`massif-current-10000/massif.444266`, SHA-256
`a778972a083b6744a28f2c2b1d3a3369826fb1111b93353c5102bbd598702998`.
[Retained samples and summary](rss-current-arm64-subset-20261005.json)
include the exact daemon/script hashes, all RSS checkpoints, and Massif
snapshot totals. That summary's SHA-256 is
`053460d23661e9cbc8ac88faaa4fbedf20041eab2129445e675113af9ef7809b`.

## Completed full blob-cycle diagnostics

A separate full blob-cycle diagnostic adds `LEASE_CREATE`, `OPEN_BLOB`,
verification and closure of the returned FD, then `LEASE_RELEASE` before
pin/unpin/GC. Its one-cycle direct and Massif smoke runs passed. The direct
10,000-cycle run and Massif 5,000-cycle run subsequently both finished with
`PROFILE_WORKLOAD_PASS` and private daemon SIGTERM status `-15`.

The direct run's RSS rose from 5,336 to 13,356 KiB; final anonymous RSS was
8,396 KiB. Its state file plateaued at 2,912,256 bytes. The Massif run ended
with 4,324,320 guest heap bytes, with a 5,099,837-byte peak across 114
snapshots. Its final RSS, including instrumentation, was 41,912 KiB.
The request mix matches the qualification runner's blob operations, but
these smaller counts do not prove an eventual ceiling. The diagnostic
does not run the runner's restart, OCI phase, or final-inventory checks.
[Retained full-cycle samples](rss-current-arm64-full-cycle-20261005.json)
have SHA-256
`be1a85c968f8106158f9952fc893df5fb0bfbb3795bc6175dd04ac0083b967c9`.

## Allocation owner and upstream repair

A separate symbol-enabled optimized build completed from the matching
private source snapshot with Cargo profile settings `inherits="release"`,
`debug=1`, and `strip=false`, using a new Cargo target directory. Its daemon
SHA-256 is
`dcbfd7834654832582da7cae71311576dcab520b510be2f8e8671e11abd28deb`,
ELF Build ID `8b8c065805aa0b6ef3b48a0c3556c2343077af13`.
Its text differs from the production release ELF, so its symbols cannot be
applied to the address-only profiles above. Its separate 5,000-cycle full
blob-cycle Massif diagnostic finished successfully and terminated its private
daemon with SIGTERM.

Across 104 snapshots, peak guest heap was 5,099,823 bytes; final signal-time
heap was 4,321,348 bytes. The peak names **2,155,008 bytes in
`VecDeque<u64>::grow` called by redb's `LRUCache<Arc<[u8]>>::insert` at
`lru_cache.rs:30`**, alongside 1,990,544 bytes in cached page contents.
The queue is an accumulating metadata owner, rather than a database-size
increase. [Retained symbolized allocation trees](rss-current-arm64-symbolized-20261005.json)
include peak and final stacks. Raw profile SHA-256:
`74e7c7ae5732bd0962453205ce2f6356a6e56a0b7ced974f0a96b0980f2c8d1a`.
Profiler RSS is not a production performance measurement.

Source inspection and an independent executable model reproduced the defect:
writing cached pages removes entries; flush re-inserts their offsets. A stale
queue token containing an offset alone then aliases a newer live entry. The
two-head pruning pass cannot distinguish them and can keep rotating stale
tokens. Several interleaved reused offsets grow the queue with constant map
size; unique offsets alone do not establish that failure.

Upstream independently repaired this defect in
[commit f95d460](https://github.com/cberner/redb/commit/f95d4609137481e6e40f04d9a103aa9541993267)
on 2026-10-01. No fixed crate release was present in the crates.io index at
inspection. The package now vendors redb 4.3.0 with that exact one-file
backport; [provenance](../vendor/redb/ARTIFACTD_PATCH.md) records the hashes.
Its generation tags distinguish old queue tokens from live entries, preserving
the mature database engine and durability policy.

Using the test-audit authoring gate, the five upstream owner tests ran first
against original 4.3.0. Mixed-churn boundedness and reinsertion eviction order
failed for the intended reasons, while three behavior guards passed. All five
pass after the exact backport. The focused artifactd CAS test also passes.
This proves the dependency regression and repair at their owner; the repaired
release still requires native workload and memory qualification. No production
bounded-memory claim is made from the smaller profiles or unit tests.
