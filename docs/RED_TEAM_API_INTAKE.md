# API intake scoped red-team review

This is an independent review of bounded Unix-socket admission in
`src/api/intake.rs` and its integration from `src/api/mod.rs`. It is not a
release approval for Artifactd.

## Reviewed source

The reviewed local source hashes are:

| File | SHA-256 |
|---|---|
| `src/api/intake.rs` | `02d08aed58b90abc9a9815e66ccba1d41ef6a9016b758bb943d0dab8fa18d9d2` |
| `src/api/mod.rs` | `25ac846cfeda6f0817193f7b840aadebe2e233210e4f86113330e851e22f9e00` |
| `tests/api_intake.rs` | `bc1b94420fb1b3da515ae35f1c399c7dbd0de9cdca0213afcde2429f6db6cffb` |

## Findings

No concrete authentication or unbounded-admission bypass was found. The
server obtains `SO_PEERCRED` immediately after `accept`, applies the loaded
peer policy before retaining the socket, limits retained sockets to 32 total
and 8 per `(uid,gid)`, and gives each retained connection a five-second first
packet deadline. Expired and errored sockets are dropped. Ready retained
requests are selected before another listener accept, preventing a connection
flood from starving an already-received request.

The public `silent_authorized_connections_do_not_delay_ready_requests` test
uses two idle authorized connections and a real daemon. It passed on both
native hosts according to the qualification run. The older serial-admission
behavior remains the negative control: idle connections delayed a ready
request until the old long packet wait. That negative control must use a
separate Cargo target directory (or a clean package build): reusing a shared
target directory can execute a cached mutant test binary after the source has
been restored, which is an evidence-harness defect rather than a production
intake defect.

One bounded availability limitation remains: `serve` executes one selected
request synchronously before returning to intake. A peer with an authorized
operation that legitimately occupies the executor can therefore delay other
peers, although the pending socket and packet admission limits remain bounded.
This is a throughput/fairness qualification gap rather than an authentication
bypass; sustained multi-peer executor fairness has not been demonstrated.

No global release approval is implied. Production caller cutover, registry and
fault campaigns, churn, and the remaining Artifactd completion gates remain
outside this scoped review.
