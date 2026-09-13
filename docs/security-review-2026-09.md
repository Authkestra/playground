# Security review of the public demo surface

**Date:** 2026-09-14
**Scope:** `main` at commit `cdf223e`. This reviews the code as it stood at that
commit — it says nothing about anything landed after it, and nothing about the
live Render/Vercel deployment beyond what the repo's own configuration files
declare.
**Tracks:** issue #34, whose six named areas this covers in full; rate
limiting, the kill switch, and the GitHub-push feature's SSRF surface were
added because a public playground for an auth framework is a uniquely
embarrassing place to get owned, and those three sit next to the named areas
in the same trust boundary.

## Verdict

The posture is strong. Session cookies are `HttpOnly` with configurable
`Secure`/`SameSite`, session ids are UUIDv4, CORS is an explicit allow-list
that never combines a wildcard with credentials, credential storage is scoped
per session and genuinely deleted — not merely hidden — on both TTL expiry and
explicit reset, error responses are a closed, typed vocabulary that never
forwards raw DB/upstream text to the client, and there is no visitor-controlled
fetch anywhere in the API, including the GitHub push feature, which only ever
calls fixed `api.github.com`/`github.com` endpoints.

Three items came out of it. One (`ProviderCredentials`'s unredacted `Debug`) is
fixed by this same change; the other two are accepted trade-offs, as issue
#34's acceptance criterion allows. One item named in the issue — confirming
SQLite credential-store expiry — does not apply to this deployment: the live
playground holds no SQLite database at all (it uses Redis, per
`docs/decisions/0004-stateless-service-on-redis.md`). SQLite only appears
inside the starter kit a visitor downloads and runs on their own machine,
outside the demo's own trust boundary, so there was nothing there to confirm.

**High: 0. Medium: 0. Low: 3 (1 fixed, 2 accepted).**

## F1 — `ProviderCredentials`'s derived `Debug` printed OAuth secrets in the clear — fixed

**File:** `apps/api/src/engine.rs:27-29`

`ProviderCredentials::creds` holds each configured OAuth provider's
`client_secret`, and the struct derived `Debug` rather than writing it by hand.
Anywhere a value built from it was ever formatted with `{:?}` — a future debug
log, an `assert!`/`panic!` message, a test failure printout — the raw secret
would land on stdout/stderr and from there in whatever aggregates the
process's logs. No call site does this today: `EngineFactory`, which holds
`ProviderCredentials`, does not itself derive `Debug`, and neither does
`AppState`. So the leak was latent, not active — but it stood alone. The rest
of the codebase is otherwise careful about exactly this: `GithubKitCredentials`
(`apps/api/src/settings.rs:27-39`), `StoredToken`
(`apps/api/src/github_token_store.rs:40-46`), and `GitHubApiError` (tested at
`apps/api/src/github_api.rs:652-671`) all hand-write `Debug` to redact
secrets, and the GitHub OAuth token additionally is never logged, per the
module's own stated invariant. `ProviderCredentials` was the one type holding
a raw secret with no such protection, and nothing stops a future debug log
statement or test assertion from formatting it.

The fix hand-writes `Debug` for `ProviderCredentials` the same way
`GithubKitCredentials` does: it prints each provider id and its `client_id`,
and redacts every `client_secret` as `"<redacted>"`. The type now matches the
rest of the codebase's own bar for a secret-bearing struct, and formatting it
is safe by construction rather than by the absence of a call site.

## F2 — Kill switch fail-open on a simultaneous cold cache and unreachable store — accepted

**File:** `apps/api/src/killswitch.rs:27-32`, `apps/api/src/killswitch.rs:219-229`

`KillSwitch::snapshot` falls back, in order, to the in-process cache, then the
store, then the last cached value, and only if there is truly no cache and the
store is unreachable, to the environment seed (`DEMO_ENABLED`, which defaults
to `true`) — the module's own doc comment says as much. A fresh instance whose
very first `snapshot()` call hits an unreachable Redis therefore serves the
environment default rather than failing closed. `open_state_store` connects to
Redis eagerly at boot, so the process never starts serving if that initial
connection fails (`apps/api/src/lib.rs:296-306`) — but a store that is fine at
boot and drops later, before the first kill-switch read populates the cache
(a Redis blip during a scale-to-zero wakeup, say), reaches exactly this
branch.

The exposure this creates: an operator flips the kill switch off as an
emergency stop, and in that same narrow window a fresh instance boots with a
cold cache and a transiently unreachable store — that instance serves live
flows as if the switch were still on. Two independent conditions have to
coincide, and the window self-corrects within the 5-second cache TTL once the
store recovers, so this is real but small.

We accept it rather than fix it. The trade-off is already the one the module
documents and defends: failing closed on this branch would mean any store
blip — not just this narrow coincidence — takes the whole site down, which is
a worse failure mode for a demo than a brief fail-open after an
already-rare emergency stop. Tightening it would mean seeding
`demo_enabled: false` specifically for the "no cache, no reachable store"
branch, at the cost of a cold boot during a genuine Redis outage serving
explainer-only mode instead of live flows — a change worth making if the
kill switch starts getting used for real, not before.

## F3 — Rate-limit bucket granularity depends on an operator step against the live deployment — accepted, with one step outstanding

**File:** `render.yaml:55-56`, `apps/api/src/settings.rs:262-281`

`TRUSTED_CLIENT_IP_HEADER` ships as `""` in `render.yaml`, with a comment
instructing the operator to determine the right header via
`GET /admin/client-ip` after deploy and set it by hand. Left empty,
`ClientIpKeyExtractor` falls back to the rightmost `X-Forwarded-For` entry —
safe and unforgeable, verified by `apps/api/src/lib.rs`'s
`key_extractor_tests` — but Render's stack likely has more than one hop in
front of the app, so the rightmost entry may be an internal proxy address
shared by every visitor. Until the header is set, the per-visitor rate limit
collapses into one shared bucket for the whole service.

This is not a bypass — `settings.rs:262-281`'s own comment calls it "coarse,
never a bypass" — and we accept it as a deployment-configuration step rather
than treat it as a code defect, because the code already defaults to the safe
side of the two wrong answers (reading the proxy's own address, not a
client-controlled one). What the review could not do is close the loop: the
live value of `TRUSTED_CLIENT_IP_HEADER` in the Render dashboard is
`sync: false` and isn't in the repo, so whether the operator step has been
done is unverifiable from the code alone.

**Outstanding action for the maintainer, against the live deployment, not the
repo:** run

```sh
curl -s -H "Authorization: Bearer $ADMIN_TOKEN" https://<api>/admin/client-ip | jq
```

and set `TRUSTED_CLIENT_IP_HEADER` in the Render dashboard from the answer
(`docs/deployment.md`'s "Settling the client-IP question" walks through
reading the result). Until that's done, the rate limiter works, just coarsely.

## Checked and found sound

### Cookie flags and session-id entropy

`apps/api/src/session.rs:28-31` (`ak_demo` cookie, 12h default TTL) and
`apps/api/src/routes.rs:97-113` (`resolve_session`): the cookie is always
`HttpOnly` (line 99); `Secure` follows `Settings::cookie_secure`
(`COOKIE_SECURE`, `true` in `render.yaml:30-31`); `SameSite` is `None` for a
genuinely cross-site deployment (Vercel + Render) or `Lax` locally, computed by
`CookieSameSite::from_env` (`apps/api/src/settings.rs:169-204`), which logs an
error if `SameSite=None` is ever set without `Secure` — browsers silently drop
such a cookie. Session ids are `Uuid::new_v4()` (`apps/api/src/session.rs:102`)
— 122 bits, not guessable. The GitHub-push CSRF-state cookie
(`apps/api/src/github_routes.rs:228-241`) and the OAuth-mode cookie
(`apps/api/src/oauth_routes.rs:117-123`) both set the same flags with short
(15-minute) `max_age`s appropriate to their purpose.

### CORS

`apps/api/src/lib.rs:468-491`: origins come only from `ALLOWED_ORIGINS`,
parsed into an explicit `Vec<HeaderValue>` and passed to
`CorsLayer::allow_origin` — never `AllowOrigin::Any`/`"*"`, so the
wildcard-plus-credentials misconfiguration this area exists to catch is
structurally impossible here; `allow_credentials(true)` next to a concrete
list is exactly the safe combination. Methods are restricted to
`GET`/`POST`, allowed headers to `Content-Type`/`Authorization`, and
`Content-Disposition` is exposed only for the starter-kit filename, which is
deliberate and self-contained. A malformed `ALLOWED_ORIGINS` entry is dropped
with a loud `tracing::error!` rather than silently widening the policy
(`apps/api/src/lib.rs:468-480`). The live value of `ALLOWED_ORIGINS` is
`sync: false` in the Render dashboard and outside what this review could
verify — see "What this review could not check" below.

### Secret handling

No `tracing::*!` call in `apps/api/src` interpolates a raw secret, token,
password, or private key: every call that names one logs either a
boolean/reason (`settings.rs:50-57`, `61-63`) or an already-redacted value
(`credentials.rs:233` logs `storage_id`, never the secret). `github_token_store.rs`,
`github_routes.rs`, and `github_api.rs` all take the token by `&str` and never
place it in a `Debug`-derived struct or a tracing field, verified by the
explicit tests `debug_output_never_carries_the_token`
(`github_token_store.rs:156-162`) and
`debug_output_never_carries_a_bearer_token` (`github_api.rs:655-670`). The
starter-kit archive never ships a filled-in secret — `.env.example` always
writes required variables empty (`apps/api/src/kit/mod.rs:1164-1170`, tested
at `kit/archive.rs:144-166` and `kit/mod.rs:1431-1460`) — and no `.env` file is
ever included. The web bundle exposes only `NEXT_PUBLIC_API_BASE_URL`
(`apps/web/lib/api.ts:13-14`, `apps/web/.env.example`), a public base URL, not
a credential. F1 was the one gap in this area, and it is now closed.

### Session isolation

Every session-scoped read or write resolves the session id from the `ak_demo`
cookie via `resolve_session` (`apps/api/src/routes.rs:87-116`) or the
equivalent inline cookie read in `oauth_routes.rs:176-179` and
`github_routes.rs:177-182` — never from a path parameter, query parameter, or
request body. `configure_scenario` and `scenario_action`
(`apps/api/src/routes.rs:214-299`) take a scenario *id* from the path, but
that names a definition in the shared registry, not another visitor's data;
the session itself always comes from the cookie. Credential storage keys every
row by `cred:{session_id}:{cred_type}:{credential_id}`
(`apps/api/src/credentials.rs:54-56`) and is tested directly for cross-session
isolation (`credentials_are_scoped_to_their_session`, `credentials.rs:420-450`);
ceremony state is keyed and tested the same way (`ceremony.rs:210-233`); the
GitHub push token is keyed by session id and tested identically
(`one_sessions_token_is_invisible_to_another`,
`github_token_store.rs:133-140`). No handler accepts a session id, credential
id, or user id from the request and uses it to reach another visitor's data.

### Expired-session credential cleanup

The issue asks this as "confirm SQLite expiry," but the deployment's actual
store is Redis/in-memory KV (`apps/api/src/store.rs`,
`apps/api/src/credentials.rs`), not SQLite — SQLite exists only inside the
generated starter kit a visitor downloads and runs themselves
(`apps/api/src/kit/mod.rs:629-653`), outside this deployment's own runtime, so
the literal question doesn't apply here. For the store actually in use: TTL
expiry is enforced by the backend itself (Redis `SET EX`/`GETDEL`,
`apps/api/src/store.rs:171-200`; `MemoryKv` enforces the same semantics on
read, `store.rs:296-337`), so an expired key is genuinely gone rather than
merely hidden from a query — verified by `an_expired_session_is_unreachable`
(`session.rs:247-266`) and `an_expired_token_is_unreachable`
(`github_token_store.rs:143-154`). Explicit reset deletes rows immediately
rather than waiting on TTL: `DemoSessionStore::reset` calls
`credentials.purge_session` (`session.rs:160-177`), which reads every
credential under the session's prefix, deletes each one and its
cross-reference index entry, and is tested for exactly that
(`purging_also_clears_the_back_references`, `credentials.rs:452-469`).

### Error responses

`apps/api/src/error.rs` defines a closed `ApiError` enum; every variant's
`(StatusCode, code, detail)` is hand-written prose or a passthrough of an
already-classified, non-sensitive `GitHubApiError`/`StoreError` message
(`error.rs:169-192`) — never a raw `anyhow`/`sqlx`/`redis` error string, a
stack trace, or an internal file path. `StoreError` maps to
`ApiError::StateUnavailable` with the store's own `Display` text, which for
`RedisKv`/`MemoryKv` carries only a driver-level connection message
(`store.rs:23-29`), never credentials or data. `GitHubApiError::Network`'s
`Display` is documented and structured to include only the transport failure,
never what was sent (`github_api.rs:565-571`). Every server error is logged in
full server-side via `tracing::error!` before the client-facing structured
JSON goes out (`error.rs:194-204`), so operators keep full detail while the
client sees only the closed vocabulary.

### Rate limiting

Two `tower_governor` buckets exist: a standard one (30 burst / 2s refill) for
ordinary endpoints, and a tighter one (10 burst / 5s refill) for anything
that creates credentials or reaches a third party — ceremony actions, the
starter-kit download, both OAuth flows, and the GitHub push action itself
(`apps/api/src/lib.rs:449-506`). Every expensive or quota-burning endpoint this
review found sits on the tighter bucket; nothing third-party-calling or
credential-creating was left on the standard one. `ClientIpKeyExtractor` is
built to resist the classic XFF-spoofing bypass — it reads the rightmost,
proxy-written entry by default, not the client-controlled leftmost one, with
tests asserting this directly (`lib.rs:540-572`). F3 above is the one
operational caveat on top of an otherwise sound design.

### Kill switch

`apps/api/src/killswitch.rs`: durable (stored in the shared KV store, survives
restarts and scale-to-zero), cached for 5s to avoid a store round trip per
request, and every write path (`admin_kill_switch` in `routes.rs:322-355`) is
gated by `authorize_admin`, which requires `ADMIN_TOKEN` to be set at all — a
missing token means the whole `/admin` router is never mounted
(`lib.rs:508-514`), never an open switch. The `admin_client_ip` diagnostic
endpoint sits behind the same gate and is documented as echoing request
headers, which is why it must stay non-public (`routes.rs:357-367`). The admin
bearer-token comparison is a plain `==`, not constant-time
(`routes.rs:316`) — worth naming, though a shared-secret bearer token compared
this way is a minor theoretical timing side-channel, and this review does not
treat it as a finding given the admin surface is optional, operator-controlled,
and the token is high-entropy. The CSRF-state comparison in the GitHub push
flow, by contrast, does use a constant-time compare
(`github_routes.rs:141-151`), which is the case that actually matters, since
that value is attacker-observable via a redirect. F2 above is this area's one
real finding.

### GitHub push feature: SSRF, token storage, scope

`apps/api/src/github_api.rs`, `github_push.rs`, `github_routes.rs`,
`github_token_store.rs`: every outbound call target is a hardcoded constant
(`GITHUB_API_BASE = "https://api.github.com"`,
`GITHUB_OAUTH_BASE = "https://github.com"`, `github_api.rs:46-47`) — no
visitor input ever becomes a request URL or host. The only visitor-supplied
values that reach GitHub are the repository name (validated by
`is_plausible_repo_name`, `github_routes.rs:189-197`, then GitHub's own
validation) and file contents generated server-side from the visitor's own
scenario selection; neither can redirect the request elsewhere. The token is
requested with the narrowest usable scope (`public_repo`,
`github_routes.rs:51`), deliberately a separate OAuth app from the sign-in
scenario's own credentials (module docs, `settings.rs:5-16`), stored
server-side keyed by session id with a 15-minute TTL
(`github_token_store.rs:26-28`), and cleared immediately after one push
attempt regardless of outcome rather than left to expire
(`github_routes.rs:413-418`). The redirect target after the OAuth round trip
is always the first configured `ALLOWED_ORIGINS` entry, never anything from
the request, so this cannot become an open redirect
(`github_routes.rs:153-169`, same pattern in `oauth_routes.rs:75-86`).

### General SSRF surface

Every `reqwest`/HTTP client construction and every `Url::parse` in
`apps/api/src` was checked. The only outbound HTTP calls are: the GitHub
endpoints above; each captcha provider's fixed `siteverify` endpoint, selected
by a closed enum (`turnstile`/`hcaptcha`/`recaptcha`) never by visitor input
(`apps/api/src/scenario/captcha.rs`); the resource scenario's own JWKS fetch,
which targets this deployment's own published `jwks_url` derived from
`PUBLIC_BASE_URL` (`signing.rs:126`), not a visitor-supplied value; and the
`url::Url::parse` calls in `scenario/passkeys.rs`, which parse the server's
own configured `WEBAUTHN_ORIGIN`/`WEBAUTHN_EXTRA_ORIGINS`, not a request
value. No handler anywhere accepts a URL or host from a visitor and
dereferences it. There is no SSRF surface in this API.

## What this review could not check

This review reads the repository at `cdf223e`; it cannot see the running
Render or Vercel deployment, and two live settings that gate the findings
above are marked `sync: false` in `render.yaml` — set by hand in the Render
dashboard and absent from the repo entirely:

- **`ALLOWED_ORIGINS`.** The code path is sound (see CORS, above), but
  whether the deployed value is actually the frontend's real origin, with no
  trailing slash and the right scheme, is only checkable against the live
  service.
- **`TRUSTED_CLIENT_IP_HEADER`.** F3 above is precisely this: the code
  defaults safely, but whether the operator has run `GET /admin/client-ip`
  and set the header is a live-deployment question, not a repository one.

Two further things followed from that same boundary rather than from any gap
in the code:

- **SQLite credential-store expiry**, as the issue names it, is not
  something this deployment's runtime does at all — it runs on Redis. The
  question only has an answer inside the starter kit a visitor generates and
  runs themselves, which is a different trust boundary than the one this
  review covers.
- **Whether any future code change reintroduces a raw `{:?}` of
  `ProviderCredentials`, or of anything else holding a secret**, is not
  something a point-in-time review can rule out going forward — it can only
  say that no call site does so at `cdf223e`, and that the type itself no
  longer makes it free to add one.
