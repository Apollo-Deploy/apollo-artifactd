# Red-team review: protocol client identity binding

Date: 2026-10-05  
Scope: `crates/artifactd-protocol/src/client.rs`, CLI `--server-uid`, and the
daemon Unix-socket accept path. This is a scoped implementation review, not a
release gate.

## Verdict

`APOLLO_ARTIFACTD_PRODUCTION_PARTIAL`. I found no confirmed defect in the
current same-UID peer binding or descriptor-send ordering. This change is a
client-side prerequisite for selecting the expected service UID; it is not a
cross-UID service identity or authorization policy.

## Reviewed behavior

- `Client::call` connects first, reads `SO_PEERCRED`, and rejects an unexpected
  server UID before cloning an input descriptor or sending request bytes. A
  wrong-UID endpoint therefore cannot receive the request or credential FD
  through this client path.
- The daemon independently checks the connecting peer UID before reading the
  packet. A client cannot bypass the server-side same-UID admission by choosing
  `--server-uid`; that option only changes which peer UID the client trusts.
- The request and response operation IDs, protocol version, and response
  operation identity are checked. Allocated mutation tokens are obtained from
  the same peer-checked client call, so allocation is bound to the selected
  socket endpoint and its server response.
- The connected Unix socket endpoint is kernel-bound after `connect`; replacing
  the pathname after connection cannot substitute a different server for that
  request. The received descriptor is returned as an `OwnedFd`, and the input
  descriptor is cloned only after peer verification.
- The default convenience client preserves same-UID behavior by using the
  caller's effective UID. The CLI default has the same behavior; an explicit
  UID is an operator trust selection, not an authorization grant.

## Evidence boundary and remaining work

Source inspection found no mandatory scoped finding. The new tests provide
useful authoring-gate coverage: wrong expected UID rejects before the fixture
observes a packet (including an input descriptor), valid UID requests receive
the response, and CLI automatic allocation uses the same check. The fixture is
not the daemon's own UID admission path, however. The ignored root-only case
is the material boundary test: it must run against the kernel's real
cross-UID behavior, with the configured UID comparator and the default-UID
rejection both exercised; synthetic fixture results must not be counted as
that proof.
The revised root-only test now copies the CLI into a root-owned traversable
fixture, so the nobody child can exec it without changing host home
permissions. Its first comparator deliberately accepts a root peer when
`--server-uid 0` is supplied; that is valid client configuration coverage, not
proof that the daemon authorizes a cross-UID caller. The second case checks
default-UID rejection.

Same-UID trust still permits any cooperating process with that UID, and the
implementation does not claim a stronger service identity mechanism. Full
release remains blocked by the project's independent ownership,
fault/recovery, caller-cutover, and other production qualification gates.

## Native evidence disposition

ARM64 passed the focused client-identity suite, including the root-to-UID
65534 kernel case, the ordinary workspace/all-target suite, clippy with
`-D warnings`, and release builds; the 101-file source snapshot matches the
reviewed tree (`docs/client-identity-native-arm64.txt`,
`docs/client-identity-final-arm64-snapshot.json`). The controlled ARM64
pre-fix comparator run also reproduced the intended unauthorized-success
failure (`docs/client-identity-pre-fix-arm64.txt`). x86_64 now has the same
focused, real cross-UID, ordinary, clippy, release, and 101-file snapshot
evidence (`docs/client-identity-native-x86_64.txt`,
`docs/client-identity-final-x86_64-snapshot.json`). Both native jobs exited
successfully with no failed tests.
