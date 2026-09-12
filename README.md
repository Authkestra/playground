# authkestra playground

An interactive playground and starter-kit generator for
[**authkestra**](https://github.com/marcjazz/authkestra), a framework-agnostic
authentication orchestrator for Rust.

Configure auth (passkeys, TOTP, OAuth, bot protection), see the diff, test it
live, download a working Rust project.

> **This repo is not the framework.** authkestra itself — the crates, the engine,
> the adapters — lives at [`marcjazz/authkestra`](https://github.com/marcjazz/authkestra)
> and has its own roadmap. This repo is only the playground that demonstrates it
> and the generator that emits starter projects. Everything here is scoped to
> capabilities already shipped in authkestra 0.8.1.

## Status

**Live.** Backend on Render (`authkestra-playground-api` service),
frontend on [Vercel](https://playground-web-opal.vercel.app).

| Scenario | State |
| --- | --- |
| **TOTP** (authenticator app) | Working end to end — enrol by QR, verify a real code |
| **Passkeys** (WebAuthn) | Working — registration and authentication, with signature-counter tracking |
| **Resource server** (protected route) | Working — validates by fetching the issuer's published keys and matching on `kid`, so the route holds no secret |
| OAuth (GitHub / Google / Discord) | Built; waits on provider credentials |
| Bot protection (Turnstile / hCaptcha / reCAPTCHA) | Built; waits on captcha site keys |

"Built; waits on" means exactly that: the scenario, its actions, its diff and its
starter-kit fragment are all shipped and tested, and the control renders itself
unavailable with a reason until the keys are in the deployment's environment.
Adding them is configuration, not a code change. Registering them is the one job
nobody but the maintainer can do — see the P0 credentials issue.

Placeholder scenarios (`dummy_toggle`, `dummy_provider`) exist for testing but
are not registered in the live service — only the real scenarios (TOTP, passkeys,
OAuth, resource server, bot protection) are available to visitors.

`play.authkestra.com` is not wired up yet — see the open P0 issues.

## Layout

```
apps/api/            Rust (Axum) backend, deploys to Render
apps/web/            Next.js (App Router) frontend, deploys to Vercel
packages/api-types/  TypeScript types generated from the Rust API contract
docs/                playground docs, API contract, decision records
roadmap/             roadmap.json (source of truth) + GitHub issue sync
```

`apps/web` is deliberately a different stack from the framework repo's
`website/` (Astro + Starlight). The two frontends do not share tooling or
components — an accepted tradeoff for Next.js's larger ecosystem.

## Running it locally

Backend:

```sh
cargo run -p api           # http://localhost:8000
```

There is one binary and no platform-specific entrypoint: the container image in
production runs exactly this, configured entirely through environment variables.

Frontend, in a second terminal:

```sh
pnpm install
pnpm web:dev               # http://localhost:3000
```

The frontend talks to `NEXT_PUBLIC_API_BASE_URL` (default `http://localhost:8000`).
No credentials or external services are needed to run the playground — provider
scenarios simply report themselves as not configured.

### Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `PORT` | `8000` | API bind port |
| `ALLOWED_ORIGINS` | `http://localhost:3000` | Comma-separated CORS origins |
| `COOKIE_SECURE` | `false` | Mark the session cookie `Secure` (set in production) |
| `COOKIE_SAMESITE` | `none` when secure, else `lax` | **Load-bearing for a cross-site deployment.** `Lax` cookies are not sent on cross-site fetches, so if the frontend and API are on different registrable domains the session silently never persists. `None` requires `Secure`. Once both share a domain (`play.` and `api.play.`), `lax` is correct and stricter. |
| `SESSION_TTL_HOURS` | `12` | Demo session lifetime |
| `DEMO_ENABLED` | `true` | Seed value for the global kill switch. **Initial value only** — on first boot, this value is written to the store and then ignored on restarts. The stored state is authoritative and survives restarts, making the switch durable across deployments. Changes via `POST /admin/kill-switch` are persisted. |
| `DEMO_DISABLED_SCENARIOS` | — | Seed value for per-scenario disables (comma-separated scenario ids). **Initial value only** — like `DEMO_ENABLED`, this seeds the store on first boot and is then ignored on subsequent restarts. |
| `ADMIN_TOKEN` | — | Enables `POST /admin/kill-switch`. **Unset means the endpoint is not mounted at all.** |
| `OAUTH_STATE_KEY` | — | ≥32 bytes; keeps encrypted OAuth state valid across restarts |
| `OAUTH_REDIRECT_BASE` | `http://localhost:8000` | Base for provider redirect URIs |
| `TRUSTED_CLIENT_IP_HEADER` | — | Header carrying the true client IP, set by the proxy in front. **Must be one the proxy overwrites**, or the rate limiter can be bypassed by forging it. No portable default exists; set this to match your proxy (e.g. `cf-connecting-ip` behind Cloudflare). Use `GET /admin/client-ip` to verify which headers actually arrive and which to trust. Empty string (the safe default) falls back to the rightmost `X-Forwarded-For` entry. |
| `CLIENT_IP_XFF_POSITION` | `rightmost` | Which `X-Forwarded-For` entry to trust. `rightmost` is unforgeable; `leftmost` is correct only where the proxy overwrites the header. Settle it with `GET /admin/client-ip` rather than guessing. |
| `<PROVIDER>_CLIENT_ID` / `_SECRET` | — | `GITHUB_`, `GOOGLE_`, `DISCORD_`. These belong to the **sign-in** scenario and want the narrowest scope that proves identity — they are deliberately not the same app as the pair below. |
| `GITHUB_KIT_CLIENT_ID` / `_SECRET` | — | A **second, separate** GitHub OAuth app, used only to push a generated project to a visitor's repo (`public_repo` scope). Separate from `GITHUB_CLIENT_ID` on purpose: folding repo-write into the sign-in app would mean every visitor who wanted to try "sign in with GitHub" authorised write access on the same click. **Unset means the push is offered as unavailable** and the zip download — which needs no account at all — carries on unaffected. See `docs/decisions/0006-github-push.md`. |
| `PUBLIC_BASE_URL` | `http://localhost:{PORT}` | This API's own externally reachable base URL. It is the `iss` of every token signed here and the prefix of the published key set, so pointing it somewhere a validator cannot reach means tokens issue fine and then fail to validate. |
| `TOKEN_SIGNING_KEY_PEM` | generated per process | Ed25519 private key, PKCS#8 PEM (`openssl genpkey -algorithm ed25519`). Literal `\n` is unescaped, for dashboards that only take one line. **Unset means a key is generated at boot**: fine for `cargo run`, but restarts invalidate outstanding tokens and two instances publish different keys. A supplied key that will not parse is fatal rather than silently replaced. |
| `<PROVIDER>_SITE_KEY` / `_SECRET_KEY` | — | `TURNSTILE_`, `HCAPTCHA_`, `RECAPTCHA_`. **Both halves or the provider is not offered** — a site key alone renders a widget whose token nothing can spend. The site key is public and reaches the browser; the secret never does. reCAPTCHA needs a **legacy** secret key — the engine speaks classic `siteverify`, not Enterprise assessments; see `docs/deployment.md`. |
| `REDIS_URL` | — | State store. **Unset means an in-process store**: fine for `cargo run`, unsafe for more than one instance. `rediss://` for TLS. |
| `REDIS_PREFIX` | `ak_playground` | Key namespace, so deployments can share one Redis |
| `WEBAUTHN_ORIGIN` | `http://localhost:3000` | The frontend's origin, exactly as the browser sends it |
| `WEBAUTHN_RP_ID` | derived from origin | Relying-party ID. May be a **registrable suffix** of the origin (`authkestra.com` for `play.authkestra.com`), which lets the site move between subdomains without invalidating every passkey. |
| `WEBAUTHN_EXTRA_ORIGINS` | — | Additional accepted origins, comma-separated. Cannot bridge different registrable domains — the browser requires the RP ID to be a suffix of the page's own origin. |

## What a visitor gets

Each visitor gets an isolated demo session (12h TTL, id in an HttpOnly cookie).
Toggles affect only that visitor. Two people using the site at once never see
each other's configuration.

**All state lives in Redis** — demo sessions, in-flight WebAuthn challenges and
scenario credentials — so the process itself holds nothing durable. Any instance
can serve any visitor, and a deploy or an instance being recycled costs nothing.
See [`docs/decisions/0002-session-store.md`](docs/decisions/0002-session-store.md).

**Expiry is Redis's TTL, not a background task.** Sessions, ceremonies and
credentials each carry one, so they clean themselves up. There is deliberately
no sweeper: a process that is allowed to scale to zero cannot be relied on to
run anything on a timer.

## API

See [`docs/api-contract.md`](docs/api-contract.md). TypeScript types in
`packages/api-types` are **generated** from the Rust structs via `ts-rs` —
regenerate with `cargo test -p api`. CI fails if the committed output is stale,
so a response shape cannot change without the types changing with it.

## Adding a scenario

Write one module implementing `Scenario` and register it in
`ScenarioRegistry::with_builtins`. The HTTP layer, diff engine and frontend all
work from the registry, so nothing else needs a new branch — the frontend
renders its controls from `ScenarioSpec` data rather than hardcoded markup.

Multi-step ceremonies (registration, verification) go through one generic
endpoint, `POST /api/scenarios/:id/action/:action`, and the scenario declares
the steps it accepts in `ScenarioSpec::actions`. That keeps per-scenario logic
out of the HTTP layer entirely.

`apps/api/tests/conformance.rs` enumerates the registry, so a new scenario is
picked up automatically and must satisfy every shared property — it cannot ship
untested.

### WebAuthn relying party

`WEBAUTHN_RP_ID` must be the **frontend's** domain, not the API's: the ceremony
runs in the browser at the page's origin, and the browser rejects any RP ID that
is not a registrable suffix of it. Aiming it at the API host is the usual cause
of a ceremony failing with a deliberately vague browser-side error.

A passkey is bound to the RP ID that created it, so **changing domains means
every visitor re-registers** — worth knowing before `play.authkestra.com` goes
live.

## Statelessness

The service is stateless by design, which is what makes it safe to host on
infrastructure that scales to zero:

| State | Where | Lifetime |
| --- | --- | --- |
| Demo session (config) | Redis | 12h TTL, refreshed on use |
| WebAuthn challenge | Redis | 5 min TTL, single-use (`GETDEL`) |
| TOTP secret / passkey | Redis | session TTL |

Nothing is written to the container filesystem, and no background task has to be
alive for cleanup to happen.

## Safety

This is a public, unauthenticated surface that will eventually create real
WebAuthn credentials and call third-party providers, so it ships with a front
door:

- **Rate limiting** — per-IP token buckets, deliberately tighter on endpoints
  that hit third parties. Returns a JSON 429 the UI renders as a friendly note.
  The client IP comes from `TRUSTED_CLIENT_IP_HEADER` (a header the proxy
  overwrites), falling back to the **rightmost** `X-Forwarded-For` entry. Reading
  the leftmost entry — the obvious choice, and what `SmartIpKeyExtractor` does —
  is a bypass: proxies *append*, so the leftmost value is whatever the client
  sent. Set this correctly whenever the proxy in front changes.
- **Kill switch** — `DEMO_ENABLED` plus per-scenario disable, durable and
  flippable at runtime through `POST /admin/kill-switch` without a redeploy. The
  environment variables seed the state on first boot only; the stored state is
  authoritative and survives restarts. A flip takes up to 5 seconds to be seen
  everywhere (the in-process cache TTL), which is an acceptable price for an
  emergency stop that happens maybe twice a year. With flows off the site
  degrades to explainer-only mode rather than looking broken. If Redis becomes
  unreachable, the switch falls back to the last known cached value and never
  fails open.

## Deployment

Backend to **Render** (from `render.yaml`), frontend to **Vercel**
(`.github/workflows/deploy-web.yml`), both on merge to `main`. Render redeploys
whenever CI passes (`autoDeployTrigger: checksPass`), so a build that fails
formatting, linting, tests, or dependency checks cannot reach production.

The frontend deploy is owned by CI rather than by Vercel's Git integration.
It builds from the checkout and ships that artifact with `vercel deploy
--prebuilt`, so what is live is the commit CI built, and a smoke test then
fetches the served bundles and fails the run if they do not contain the
current build. Turn the dashboard Git integration off if it is still on:
otherwise both deploy on every push and race.

Shuttle was the original plan; it was dropped after the project turned out to be
abandoned — see
[`docs/decisions/0003-hosting-platform.md`](docs/decisions/0003-hosting-platform.md).

⚠️ **The free Render plan spins the service down after inactivity**, so the first
request after a quiet period pays a cold start of tens of seconds. That used to be
disqualifying, when demo sessions lived in process memory; now that all state is in
Redis it costs latency and nothing else — a visitor's configuration survives it. The
frontend says so on its loading screen, because a silent minute reads as a broken site.

Two other targets stay configured but dormant, both `workflow_dispatch` only:

- **Cloud Run** (`.github/workflows/deploy-cloudrun.yml`) — scales to zero like
  Render's free plan, but cold-starts this binary in about a second rather than ~50.
  The trade-off is that GCP wants a billing account even for the free tier.
- **Fly.io** (`.github/workflows/deploy.yml`, plus the now-unused `fly.toml`) — the
  previous platform. Its trial ended, and a push-triggered deploy that always fails
  leaves `main` red for reasons unrelated to the commit, so the trigger was removed
  rather than the workflow. Making Fly the target again is a card and a `push` trigger.

### Why the frontend deploy moved into CI

`apps/web/vercel.json` used to set an `ignoreCommand` so backend-only commits
would not rebuild the frontend:

```
git diff --quiet HEAD^ HEAD -- . ../../packages
```

It inspects **only the tip commit of a push**, and Vercel builds only that tip.
So a frontend change pushed as anything other than the last commit is skipped —
and because the check never looks further back, no later push repairs it. The
change stays undeployed indefinitely while every build reports success.

That is not hypothetical: `4bf88da`, which rebuilt the resource-server panel on
JWKS key discovery, went up in the same push as a docs-only commit. The tip
touched no frontend files, the build was skipped, and `play.authkestra.com`
served the previous panel — flow log, symmetric sign/verify and all — for hours
with nothing reporting a problem, while the API deployed correctly and served
the new key set.

The `ignoreCommand` is gone and the deploy is a workflow. GitHub's `paths:`
filter evaluates the whole push rather than its tip, so the failure cannot
recur, and a skipped or stale deploy now shows up as a red run.

See [`docs/deployment.md`](docs/deployment.md) for the full procedure. The short
version: the API needs a Redis (`REDIS_URL`) and any container host; the
frontend needs Vercel with root directory `apps/web`; and the API's
`ALLOWED_ORIGINS` and the frontend's `NEXT_PUBLIC_API_BASE_URL` must name each
other exactly.

## CI

Mirrors the framework's bar: `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test`, `cargo llvm-cov`, `cargo deny check`, plus frontend typecheck,
lint and build. The coverage gate starts at 50% (the framework enforces 84) and
should ratchet up.

## Licence

MIT OR Apache-2.0, matching the framework.
