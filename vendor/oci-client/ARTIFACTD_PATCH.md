# artifactd `oci-client` fork

This directory is a maintained local fork of upstream `oci-client` **0.18.0**
from <https://github.com/oras-project/rust-oci-client>. The upstream Apache-2.0
license and attribution files remain in this directory. The fork is kept
independent of the artifactd workspace and has no Apollo package dependency.

## Patch scope

- Split the large upstream client and manifest modules into cohesive modules;
  no generated protocol or registry mechanisms were replaced.
- Added bounded streaming reads for manifests, authentication responses, and
  error responses. Content-Length is checked before allocation and chunked
  responses are capped cumulatively.
- Added configurable total request timeout and a bounded redirect policy.
  Secure redirect chains remain HTTPS, redirect userinfo is rejected, and the
  redirect count is finite.
- Restricted bearer realms to HTTPS by default, same registry authority, or an
  explicitly configured **exact host:port** authority. Realm userinfo is
  rejected, and supplied Basic credentials are never sent to an untrusted
  realm. Auth request failures are sanitized so query-bearing realm URLs and
  response bodies cannot appear in errors or logs.
- Removed URL-bearing request logging while retaining operational status and
  size diagnostics.
- Added public `Client` HTTP-boundary tests for cumulative chunked limits,
  total timeout, untrusted bearer realm credential isolation, and redirect
  userinfo rejection. Upload `Location` values are parsed and reject
  userinfo, unsupported schemes, and HTTPS-to-HTTP downgrade; cross-host
  upload requests remain unauthenticated.
- Fixed the upstream no-`Location` manifest workaround for digest references:
  it now derives the requested digest or tag without an unchecked `expect`.

## Test-audit evidence

Each new test observes behavior through the public `Client` API and a local
HTTP socket. The regressions are credible because the old implementation had
unbounded non-blob response sinks, no total request deadline, and redirect or
realm decisions that were only private predicates. Existing upload tests did
not exercise chunked metadata, delayed headers, bearer realm isolation, or
redirect userinfo. The tests use production configuration and real request
boundaries; they add no test-only production seam.

The HTTPS downgrade branch is enforced in the redirect policy by the initial
request scheme. The local boundary suite exercises the same policy through a
userinfo redirect; an end-to-end HTTPS downgrade server requires a test TLS
certificate and is not part of this vendored crate's current dev dependency
set.

## Validation

From this directory:

```text
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked
```

All three commands pass in the maintained fork. The full test suite includes
the upstream digest and upload-auth coverage plus the four new security-policy
boundary tests (six security-policy tests currently run).

## Pre-fix regression evidence

The public regressions were run against a temporary copy with each fix
selectively reverted. The digest-reference test failed with exit status 101
and the upstream panic `The manifest URL always ends with the image tag
suffix` at `push.rs:94`. The upload-Location test failed with exit status 101
because the reverted implementation returned `Ok("http://user:password@...")`
instead of rejecting the userinfo URL. These failures demonstrate the intended
regressions at the public `Client::push_manifest_raw` boundary rather than
private predicate behavior.
