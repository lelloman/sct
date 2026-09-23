# Steps 08/09: shared authentication and access

SCT's production server uses `simple_server::auth::HeaderCredential` to parse
the exact `Bearer ` Authorization syntax and `AsyncAccess` to authenticate
the token or browser session through SCT's existing database verifier. Browser
content-mutation denial is an access check after session verification. The core
uses `Access` for administrator and root-capability decisions after loading the
authoritative rows within the existing transaction.

The shared module does not store credentials or grants. SCT keeps its hashed
token/session lookup, revocation and expiry, principal disabled state, CSRF and
origin checks, route-specific browser restrictions, grant/ownership model,
audit writes, and status/body mapping. Authorization remains higher precedence
than a session cookie. The old parser selected the first repeated Authorization
header, accepted only exact-case `Bearer `, and left an empty token for SCT's
length check; the shared parser opts into these compatibility settings. Invalid
supplied Authorization never falls back to the cookie.

Migration base: `master` at `0c750e7a3b346ad378a4de663debff79403d7957`.
Reviewed shared source: `0a629da7b5eb5aeeb0ed64aac2c5f96cd4d9717b`;
its source and manifests match the adjacent checkout's `3eaaf66` docs-only tip.
The active `simple-server.rev` pin was advanced from
`429b782fed05cfbb43bafde3deefe535c838339c`.

Before edits, core/server test targets compiled. The nonignored suite passed
outside the sandbox; localhost tests fail with `Operation not permitted` inside
the sandbox. The existing real-HTTP management auth test passed against a
disposable PostgreSQL container before edits.

After migration, the real-HTTP management test passed with the new Bearer
syntax, repeated-header and cookie-precedence cases, as well as its existing
admin denial, credential revocation and CSRF cases. The real-HTTP storage
configuration test passed its admin boundary. The core grant-boundary test
passed its grant, revocation, stale-actor and session/issuer checks. The
core/server nonignored suite passed 18 tests (91 intentionally ignored), as did
the format check, strict affected-target Clippy, and test-target compilation.
All HTTP and database checks used the disposable PostgreSQL container; ignored
integration cases outside the auth scope were not counted as passing.
