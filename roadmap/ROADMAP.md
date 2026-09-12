# Authkestra Playground + Starter Kit — Roadmap

> **Scope.** This roadmap covers play.authkestra.com (the no-code auth playground) and the downloadable starter-kit generator ONLY. It is deliberately separate from, and subordinate to, the framework roadmap in authkestra's own docs/roadmap.md. Every deliverable here is scoped to capabilities already shipped in authkestra 0.8.0 — nothing here depends on GNAP, VC/OIDC4VP, BBS+, SSF/CAEP, ReBAC/ABAC, or PQC/ML-DSA, all of which are the framework's own future phases and explicitly out of scope for the playground.

## Phases at a glance

| Phase | Title | Issues | Goal |
|---|---|---|---|
| `P0` | Foundations | 7 | Repo, hosting, CI, and provider credentials exist and a hello-world Rust service is reachable at play.authkestra.com. No auth logic yet. |
| `P1` | Playground core | 7 | The session/state/diff/safety machinery that every scenario depends on. Still no user-visible auth flows. |
| `P2` | Scenarios | 8 | Every v0 auth capability works end-to-end against the real framework: passkeys, TOTP, OAuth (GitHub/Google/Discord), bot protection (Turnstile/hCaptcha/reCAPTCHA). |
| `P3` | Playground UI | 9 | The surface a visitor actually touches: zero-JS explainer pages plus an interactive playground island for toggling, diffing, and testing. |
| `P4` | Downloadable starter kit | 12 | The playground's configuration becomes a real, compiling Cargo project the visitor can download and run — the bridge from demo to the v0-for-Rust wizard idea. |
| `P5` | Launch hardening | 4 | Make the public surface safe, affordable, and measurable, then announce it. |
| `P6` | Post-launch / wizard path | 6 | Backlog: what turns the playground into the broader 'v0 for Rust' scaffolder, plus cratestack integration. |

**53 issues across 7 phases.** P0–P5 is v0; P6 is backlog.

## P0 — Foundations

**Goal.** Repo, hosting, CI, and provider credentials exist and a hello-world Rust service is reachable at play.authkestra.com. No auth logic yet.

**Exit criteria.** A trivial Axum endpoint deploys to Fly.io on merge, the Next.js frontend deploys to Vercel on merge, play.authkestra.com resolves through Cloudflare, and all six provider credential sets are registered and stored as secrets.

### Issues

#### Scaffold playground monorepo (apps/api, apps/web, packages/api-types)

`area:infra` `type:chore`

Create the monorepo skeleton for the playground.

```
apps/api/          # Rust (Axum) backend, deployed to Fly.io
apps/web/          # Next.js frontend, deployed to Vercel
packages/api-types # TS types generated from the Rust API contract
docs/              # playground-specific docs
```

### Tasks
- [ ] Workspace `Cargo.toml` with `apps/api` as a member
- [ ] `apps/web` initialised with Next.js (App Router) + pnpm. Note: this is deliberately a different stack from the framework repo's `website/` (Astro + Starlight), so the two frontends will not share tooling or components — accepted tradeoff for Next.js's larger ecosystem
- [ ] `packages/api-types` placeholder wired into `apps/web` tsconfig paths
- [ ] Root README stating what this repo is and that the framework lives in `marcjazz/authkestra`
- [ ] `.gitignore`, license headers consistent with the framework repo (MIT/Apache-2.0 dual)

### Acceptance
`cargo check -p api` and `pnpm --filter web build` both succeed from a clean clone.

#### Pin the authkestra dependency strategy and TLS feature choice

`area:api` `type:chore`

**This is a real trap and needs deciding before any scenario code is written.**

The `authkestra` facade crate does **not** expose `webauthn`, `totp`, `captcha`, `op`, `devsig`, or store-backend features — those exist only on `authkestra-engine` / `authkestra-axum` / `authkestra-store-sqlx`, and the facade pulls them in solely as `[dev-dependencies]` for its own examples. A downstream app that wants passkeys, TOTP or CAPTCHA must depend on those crates **directly** with their own feature names.

Second trap: the TLS backend is a mandatory either/or forwarded through every HTTPS-speaking crate. `rustls-aws-lc-rs` (the default) needs a C toolchain; `rustls-no-provider` requires the app to install a `rustls::CryptoProvider` before any HTTP client is constructed, or `reqwest` panics at construction. Cargo features being additive means one stray dependency can drag `aws-lc-rs` back in.

### Tasks
- [ ] Write the exact dependency block for `apps/api` (direct deps: `authkestra-engine`, `authkestra-axum`, `authkestra-providers`, `authkestra-store-sqlx` as needed) with explicit feature lists
- [ ] Choose the TLS backend for both container builds and generated starter kits; document why
- [ ] Verify with `cargo tree -i aws-lc-rs -e features` that the chosen backend is what actually ends up in the graph
- [ ] Record the decision in `docs/decisions/` so the starter-kit generator emits the same thing

### Acceptance
A documented, verified dependency + feature baseline that the generator can emit verbatim.

#### Decide the session store: Redis vs Memory (generic SQL session storage no longer exists)

`area:api` `area:infra` `type:chore`

**Corrects an earlier planning assumption.** The framework's generic `SqlKvStore` was *deleted* in the Epic #289 storage rework — not deprecated. SQL storage is now scoped to two specific uses only:

- `SqlxCredentialStore` (WebAuthn/TOTP credentials) — lives in `authkestra-engine`, gated by `sql-sqlite` / `sql-postgres` / `sql-mysql`
- `SqlxOpStore` (OAuth/OIDC-provider tables) — the separate `authkestra-store-sqlx` crate

There is no "put the session blob in Postgres" primitive any more. Generic session KV is **Memory or Redis**.

### Options
- **Memory**: zero infra, but demo sessions die on every redeploy and can't scale past one instance
- **Redis**: survives redeploys, scales, costs money and adds a dependency

Given the 12h TTL and that losing in-flight demo sessions on a deploy is a minor annoyance, Memory may be acceptable for v0 — but it must be a conscious decision, not a discovery in production.

### Tasks
- [ ] Decide Memory vs Redis for v0 (and note the migration path)
- [ ] If Redis: pick the provider, wire the secret, confirm the cost fits the guardrails
- [ ] Confirm `SqlxCredentialStore` + SQLite is what backs passkey/TOTP credentials regardless of the session choice
- [ ] Document the choice and its redeploy behavior in the README

### Acceptance
Decision recorded, store wired, redeploy behavior documented and expected.

#### Set up Fly.io app and deploy-on-merge for apps/api

`area:infra` `type:chore`

Deploy the Rust backend to Fly.io from CI.

**Platform changed from Shuttle.** Shuttle was the original plan but the project is abandoned — `shuttle-hq/shuttle` is archived and `shuttle-runtime` has not been published since 2025-09-11. See `docs/decisions/0003-hosting-platform.md`.

### Tasks
- [ ] Create the Fly app (`fly launch --no-deploy` reuses the committed `fly.toml`)
- [ ] Keep `auto_stop_machines = false` and `min_machines_running = 1` — these are load-bearing, not cost tuning: demo sessions live in process memory and the sweeper is a `tokio::interval`, so scale-to-zero resets every visitor's config and a second machine serves inconsistent state
- [ ] GitHub Actions workflow: deploy on merge to `main` (needs the `FLY_API_TOKEN` repo secret)
- [ ] Store all provider secrets via `fly secrets set`, never in the repo
- [ ] Note that the container filesystem is ephemeral, so P2's SQLite credential store will not survive a redeploy — acceptable given the 12h TTL, but P2 should expect it

### Acceptance
A trivial `/health` endpoint is live on the Fly.io URL and redeploys automatically on merge.

#### Set up Vercel project and play.authkestra.com DNS through Cloudflare

`area:infra` `type:chore`

### Tasks
- [ ] Vercel project with root directory `apps/web`, ignored-build-step so unrelated commits don't rebuild
- [ ] `play.authkestra.com` DNS record; Cloudflare proxy enabled for free DDoS/bot mitigation
- [ ] API subdomain (e.g. `api.play.authkestra.com`) CNAME'd to Fly.io, also Cloudflare-proxied
- [ ] Preview deployments enabled for PRs
- [ ] Grant the co-contributor admin on both Vercel and Fly.io (avoid a single point of failure)

### Acceptance
play.authkestra.com serves the Next.js app over HTTPS; the API subdomain reaches Fly.io; both are Cloudflare-proxied.

#### Mirror the framework's CI quality bar in the playground repo

`area:infra` `type:chore`

The framework repo enforces a specific bar; the playground should not be visibly sloppier than the thing it advertises.

Framework CI runs:
```
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info --fail-under-lines 84
cargo deny check
```

### Tasks
- [ ] Port fmt/clippy/test/deny to the playground repo (coverage threshold can start lower but should be explicit)
- [ ] Add `deny.toml` consistent with the framework's
- [ ] Frontend CI: typecheck, lint, build
- [ ] Note that no MSRV is pinned upstream (CI uses `dtolnay/rust-toolchain@stable`) — decide whether the playground pins one

### Acceptance
CI fails on formatting, clippy warnings, test failures, and advisory/license violations.

#### Register OAuth apps and captcha site keys for play.authkestra.com

`area:infra` `blocked:external` `type:chore`

Six credential sets are needed before the scenarios can work end-to-end.

### Tasks
- [ ] GitHub OAuth app + redirect URI
- [ ] Google OAuth/OIDC client + redirect URI (note: consent screen verification may be required for a public app)
- [ ] Discord OAuth app + redirect URI
- [ ] Cloudflare Turnstile site key + secret
- [ ] hCaptcha site key + secret
- [ ] Google reCAPTCHA site key + secret
- [ ] All secrets stored in Fly secrets / Vercel env vars; site keys (public) can live in frontend config
- [ ] Redirect URIs registered for both production and preview domains, or a documented single-callback strategy

### Acceptance
Every provider can complete a manual round-trip against the deployed environment.

---

## P1 — Playground core

**Goal.** The session/state/diff/safety machinery that every scenario depends on. Still no user-visible auth flows.

**Exit criteria.** A visitor gets an isolated 12h demo session, can toggle a dummy scenario, sees a real config diff, and the endpoint is rate-limited with a working kill switch.

### Issues

#### Model the per-visitor demo session with a 12h TTL

`area:api` `type:feature`

Every visitor gets an isolated demo session. Toggles affect only that visitor — never global state, which would break the moment two people use the site at once.

### Tasks
- [ ] `DemoSession { id, created_at, expires_at, config }`, id in a signed/HttpOnly cookie
- [ ] 12h TTL; lazy expiry-on-read (treat stale as absent) plus a `tokio` interval sweep — the service runs as a long-lived process, so no external cron is needed
- [ ] `POST /session/reset` to start clean
- [ ] Expiry also cleans up any WebAuthn credentials / TOTP secrets created during that session
- [ ] Document what a visitor loses on redeploy (depends on the store decision)

### Acceptance
Two concurrent visitors can hold different configurations simultaneously; expired sessions are unreachable and leave no credential rows behind.

#### Implement the ScenarioSpec abstraction (boolean and provider-select shapes)

`area:api` `type:feature`

Not every control is an on/off switch. Passkeys and TOTP are boolean; OAuth and bot protection are "pick a provider". Forcing everything into a boolean now means retrofitting later.

### Tasks
- [ ] `ScenarioSpec` describing id, human name, control shape (`Toggle` | `SelectOne { options }` | `SelectMany`), and dependencies
- [ ] Uniform per-scenario endpoints: `POST /scenarios/:id/configure`, `GET /scenarios/:id/diff`, `POST /scenarios/:id/try`
- [ ] Registry so adding a scenario is "write one module", not "extend a conditional"
- [ ] Serialize the spec list for the frontend so the UI renders controls from data, not hardcoded markup

### Acceptance
A dummy scenario can be registered, configured, diffed and 'tried' without touching shared code paths.

#### Build the config-diff engine

`area:api` `type:feature`

"See the diff" is the core promise; the diff must be real, derived from actual configuration state rather than hand-written copy.

### Tasks
- [ ] Serialize `DemoConfig` before/after a change
- [ ] Emit a structured diff (added/removed/changed) the frontend can render without re-deriving semantics
- [ ] Include the human-meaningful consequences alongside raw config: which routes appear, which requirements change, which crates/features a real project would need
- [ ] Snapshot tests so diff output doesn't drift silently

### Acceptance
Toggling any scenario yields a diff that names the concrete configuration and dependency changes it implies.

#### Construct a per-session Engine from the visitor's config

`area:api` `type:feature`

The framework uses a typestate builder (`Authkestra::builder()`) where methods only exist once their dependencies are provided. Building one engine per session config, at runtime, needs care.

### Tasks
- [ ] Map `DemoConfig` to a constructed engine (cache per distinct config where practical rather than rebuilding per request)
- [ ] Keep the host-app concerns (user/account records) owned by the playground — the framework deliberately has no user trait
- [ ] Follow the framework's stateless-OAuth convention: `state`/`nonce` in encrypted cookies, never the database
- [ ] `tracing` instrumentation on every handler and branch, matching the framework's definition of done

### Acceptance
Each session's flows execute against an engine built from that session's configuration, with no cross-session leakage.

#### Add rate limiting to all demo endpoints

`area:api` `type:security`

This is a public, unauthenticated surface that creates real WebAuthn credentials, real TOTP secrets, and makes real calls to third-party providers. It needs a front door.

### Tasks
- [ ] Per-IP and per-session token-bucket limits (`tower-governor`) on `configure` / `try` endpoints
- [ ] Tighter limits on endpoints that hit third parties (OAuth, captcha siteverify) to protect provider quotas
- [ ] Return clear 429s the UI can render as a friendly message
- [ ] Load-test the limits before launch (see P5)

### Acceptance
Scripted abuse of the try endpoints is throttled without degrading normal interactive use.

#### Add a kill switch for demo flows

`area:api` `area:infra` `type:security`

A way to disable live flows instantly, without a redeploy, if something is being abused or a provider quota is burning.

### Tasks
- [ ] Global `DEMO_ENABLED` flag plus per-scenario disable
- [ ] Flag readable without a redeploy (env + runtime refresh, or a small admin endpoint behind a secret)
- [ ] Frontend degrades gracefully to explainer-only mode when flows are disabled

### Acceptance
Flipping the switch stops all live flows within seconds and the site still reads as intentional rather than broken.

#### Generate TypeScript types from the Rust API contract

`area:api` `area:web` `type:chore`

Hand-maintained duplicate interfaces will drift. Generating them also makes a small, honest point about what typed Rust buys you.

### Tasks
- [ ] Pick the mechanism (`ts-rs`, or `utoipa` → OpenAPI → `openapi-typescript`)
- [ ] Emit into `packages/api-types`
- [ ] CI check that generated types are up to date (fail if regenerating produces a diff)

### Acceptance
Changing a Rust response shape without regenerating types fails CI.

---

## P2 — Scenarios

**Goal.** Every v0 auth capability works end-to-end against the real framework: passkeys, TOTP, OAuth (GitHub/Google/Discord), bot protection (Turnstile/hCaptcha/reCAPTCHA).

**Exit criteria.** Each scenario can be enabled, produces a meaningful diff, and completes a real flow — verified by integration tests covering happy path and the main failure paths.

### Issues

#### Passkeys scenario (WebAuthn registration + authentication)

`area:scenario` `area:api` `type:feature`

The flagship demo. WebAuthn is shipped in `authkestra-engine::auth::webauthn` behind the `webauthn` feature (`webauthn-rs`), with signature-counter tracking for clone detection, and works as primary or step-up. `crates/authkestra/examples/axum_mfa_server.rs` and `crates/authkestra-engine/examples/totp_webauthn.rs` are the reference wiring.

### Tasks
- [ ] Registration ceremony end-to-end against a per-session ephemeral user
- [ ] Authentication ceremony, including counter tracking
- [ ] Credentials persisted via `SqlxCredentialStore` (SQLite), cleaned up on session expiry
- [ ] Relying-party ID configured correctly for `play.authkestra.com` (and documented for preview domains)
- [ ] Diff output naming the real crates/features a project would need (`authkestra-engine` with `webauthn`, not the facade)

### Acceptance
A visitor on a supported device registers and then authenticates with a passkey, and the diff accurately describes what changed.

#### Passkey capability detection and fallback UX

`area:scenario` `area:web` `type:feature`

A meaningful share of visitors cannot complete a WebAuthn ceremony — locked-down corporate machines, unusual browsers, no platform authenticator. A dead end there reads as "this framework is broken", which is the opposite of the point.

### Tasks
- [ ] Feature-detect platform authenticator availability before offering the flow
- [ ] Explicit, friendly fallback state explaining why and offering the other scenarios
- [ ] Handle mid-ceremony abort/timeout without leaving orphaned state
- [ ] Verify against at least one deliberately unsupported environment

### Acceptance
An unsupported visitor gets a clear explanation and a working alternative, never a spinner or a stack trace.

#### TOTP scenario with QR provisioning

`area:scenario` `area:api` `type:feature`

`TotpAuthMethod` is shipped in `authkestra-engine::auth::totp` behind the `totp` feature (`totp-rs`).

### Tasks
- [ ] Per-session secret generation and provisioning URI
- [ ] QR code rendering (decide server-rendered vs client-rendered)
- [ ] Verification endpoint with clear success/failure states and clock-skew tolerance
- [ ] Secret stored via `SqlxCredentialStore`, destroyed on session expiry
- [ ] Decide whether to offer an in-page code generator so visitors without an authenticator app can still try it

### Acceptance
A visitor scans the QR with a real authenticator app and completes verification; an invalid or stale code fails cleanly.

#### OAuth scenario across GitHub, Google and Discord

`area:scenario` `area:api` `type:feature`

Shipped as `authkestra-providers` with `github` / `google` / `discord` features. The provider files are thin endpoint config (~20-40 lines each) — the flow logic is the generic `OAuth2Flow` in the engine — so this is one implementation with three configurations, not three features. Google goes through `authkestra-oidc`. Reference examples: `axum_oauth2_github.rs`, `axum_oauth_stateless.rs`, `axum_oidc_google.rs`.

### Tasks
- [ ] Provider-select control wired to the three configured providers
- [ ] Stateless variant demonstrated too (encrypted-cookie `state`/`nonce`, no session store) since it's a genuine selling point
- [ ] Callback handling for all three, with clear errors on denial/cancel
- [ ] Diff naming the exact feature flags per provider
- [ ] Explicitly out of scope for v0: letting visitors supply their own client credentials (decided against — not worth the secret-handling exposure)

### Acceptance
All three providers complete a real round trip and land the visitor in an authenticated demo session.

#### Bot-protection scenario (Turnstile, hCaptcha, reCAPTCHA)

`area:scenario` `area:api` `type:feature`

Shipped as a single `CaptchaVerifier` with a `CaptchaProvider` enum (`Turnstile`, `HCaptcha`, `ReCaptcha`) in `authkestra-engine::captcha` behind the `captcha` feature; each hits its real `siteverify` endpoint. The Axum adapter's `captcha` feature just forwards to the engine — there is no middleware and no extractor, so the check stays visible in the handler that makes it.

### Tasks
- [ ] Provider-select control across the three
- [ ] Render the correct widget per provider on the frontend (each has its own script and site-key handling)
- [ ] Mount the widget per provider in the sign-in flow tester — moved here from **Flow tester components for each scenario** (P3), where it sat as one bullet among five and the dependency below was invisible
- [ ] Show the verification result, including a deliberate failure path so the difference is visible
- [ ] Diff explaining where the check sits in the request lifecycle
- [ ] Starter-kit fragment putting the check at the top of a handler rather than on a route of its own

### Depends on
The captcha half of **Register OAuth apps and captcha site keys for play.authkestra.com** (P0) — Turnstile, hCaptcha and reCAPTCHA site keys and secrets registered against `play.authkestra.com`. That is the one piece nobody but the maintainer can do.

It gates the *acceptance criterion*, not the work. Everything else ships without the keys: the scenario, both actions, the diff, the starter-kit fragment and the widget panel all exist, and the control renders itself unavailable with a reason until keys are present — the same shape the OAuth scenario uses. Putting the keys in the deployment's environment is the whole of what remains, with no code change.

### Acceptance
Each of the three providers verifies a real token against its live siteverify endpoint, and a failed verification is demonstrable.

#### Resource-server scenario (validate a token on a protected route)

`area:scenario` `area:api` `type:feature`

`authkestra-resource` ships in 0.8.x and the playground never mentions it exists. Being able to *validate* tokens — not just issue them — is a capability most auth libraries leave to the application, and the playground currently undersells it by omission.

It was not left out deliberately: P2 was framed as "login methods", and a resource server is a server role rather than a way to sign in, so it never got a slot. There is no ADR recording the exclusion.

It demos well, which is why this one is worth doing before the OP server: issue a token, call a protected route with it, call the same route without it, watch the 401. That is a complete story with no second application required.

### Tasks
- [ ] Toggle that mounts a protected route on the demo API
- [ ] Show the visitor their current token, and let them call the route with and without it
- [ ] Surface the failure modes distinctly — absent, malformed, expired, wrong audience — rather than a flat 401
- [ ] Flow-log entries for each outcome, as the other scenarios have
- [ ] Diff naming `authkestra-resource` and the adapter's `resource` feature
- [ ] Starter-kit fragment so a downloaded project has the protected route too

### Acceptance
A visitor can call a protected route with a valid token and get through, and can see each distinct rejection reason without guessing.

### Not in scope
The OP server (`authkestra-op`). Demonstrating it properly needs a client application to complete a flow against, so it belongs on its own page rather than as a step-1 toggle. Worth a separate issue once this lands.

#### Scenario conformance tests

`area:scenario` `type:test`

Every scenario exposes the same three-endpoint contract, so it should be tested uniformly rather than ad hoc.

### Tasks
- [ ] Shared test harness driving configure → diff → try for every registered scenario
- [ ] Happy path plus the main failure paths (denied OAuth, wrong TOTP code, failed captcha, aborted WebAuthn)
- [ ] Session isolation test: two sessions with conflicting configs don't interfere
- [ ] Expiry test: an expired session's credentials are gone

### Acceptance
Adding a scenario without wiring it into the harness fails CI.

#### Rebuild the resource-server scenario on authkestra-resource and JWKS

`area:scenario` `area:api` `type:feature`

The shipped resource-server scenario mints a token with `TokenManager` and a **per-session HMAC secret**, then validates it in the same process. Symmetric, self-referential, no `kid`, no key discovery. It demonstrates that a 401 can be produced, not that a resource server works — nothing is federated, and the "server" holds the same secret that issued the token.

This was not a judgement call at the time. The original issue's own task list said "Diff naming `authkestra-resource` and the adapter's `resource` feature". The implementation named `authkestra-engine`'s `token` feature instead, and the issue was closed without the deviation being recorded.

### What the framework already provides
`authkestra-resource` 0.9.2, behind `authkestra-axum`'s `resource` feature, is considerably more than the current scenario uses:

- `Jwks::fetch` / `JwksCache` — keyed by `kid`, with a refresh interval and a `require_kid` strict mode
- `IssuerTrustMap` — `iss` -> JWKS endpoint, so a token from an unlisted issuer is rejected outright rather than falling back to a default key
- `ValidationConfig` builder — `jwks_url`, `issuer`, `trusted_issuers`, `audience`, `algorithms`, `require_kid`, `require_cert_binding`, `require_dpop`, `dpop_resource_origin`
- `JwtStrategy`, and a `Guard` with `AuthPolicy` for chaining strategies
- DPoP (RFC 9449) proof verification with `jti` replay tracking, and RFC 8705 certificate binding

And on the issuing side, `authkestra-engine` already has what is needed to be discovered: `TokenManager::new_ed25519` (and `new_asymmetric` for RS256) sets a `kid` header and exposes `public_jwk()`, ready to serve as a JWKS.

### The shape to build
The playground gains a **per-deployment** signing key (asymmetric, Ed25519) and publishes `GET /.well-known/jwks.json`. The scenario's `call` step validates through `authkestra-resource`, fetching that JWKS and matching on `kid` — so the validating side holds no secret at all, only a URL. That is the capability, and it is the part every library's documentation asserts rather than shows.

- [ ] Per-deployment Ed25519 signing key: `TOKEN_SIGNING_KEY_PEM` from the environment, generated at boot when absent
- [ ] `GET /.well-known/jwks.json` publishing the public half, so a visitor can fetch it themselves and check a `kid` by hand
- [ ] `call` validates via `JwksCache` + `ValidationConfig`, not a shared secret
- [ ] Diff naming `authkestra-resource` and the adapter's `resource` feature — the task the original issue asked for
- [ ] Starter-kit fragment emitting a real JWKS-validating resource server

### The failure modes this makes reachable
The current six (absent, malformed, expired, wrong audience, bad signature, accepted) are the easy half. JWKS validation adds the ones that actually cost people afternoons:

- [ ] **Unknown `kid`** — signed by a key that is not in the JWKS. Checkable by hand: fetch the JWKS and look
- [ ] **Untrusted issuer** — an `iss` absent from the trust map, rejected rather than guessed at
- [ ] **Missing `kid`** — refused under `require_kid`, which is the strict policy worth defaulting to
- [ ] **Stale cache** — a key rotated at the issuer but not yet refreshed at the resource server

Each needs a way to mint a deliberately-wrong token. That stays honest as long as the token is shown and editable and the verdict comes from real validation — a forgery a visitor can verify is not a forgery they have to take on trust.

### Why this comes before the OP page
"Be your own identity provider: an OP server page" (P6) records its own blocker as needing a client to complete a flow against. A JWKS-validating resource server **is** that counterpart: the OP issues and publishes keys, the resource server discovers them and validates. Two scenarios, one real trust relationship, and a demo that ends in "rotate the OP's key and watch the resource server pick it up".

Built the other way round, the OP page has nothing to prove itself against and the resource server stays a toy.

### Scope
Replaces the current scenario rather than sitting beside it. The symmetric path is not worth keeping as a second option: it is the thing this issue exists to stop demonstrating.

Not in scope here: DPoP and certificate binding. Both are supported by the crate and both deserve showing, but they are a second scenario — proof-of-possession is a different lesson from key discovery, and stacking them would make neither legible.

### Acceptance
A visitor issues a token, sees the `kid` in its header, fetches `/.well-known/jwks.json` and finds that same `kid`, and watches the protected route accept it. Then they see a token signed by a key absent from that JWKS rejected for that reason by name, and an untrusted issuer rejected for its own.

---

## P3 — Playground UI

**Goal.** The surface a visitor actually touches: zero-JS explainer pages plus an interactive playground island for toggling, diffing, and testing.

**Exit criteria.** Full toggle → diff → test loop is usable on desktop and mobile, in light and dark, passes an a11y review, and meets the agreed Lighthouse budget in CI.

### Issues

#### Next.js app shell with minimal-JS explainer routes

`area:web` `type:feature`

Next.js App Router, chosen over Astro for ecosystem depth despite authkestra.com running Astro/Starlight (the two frontends stay separate stacks by decision).

Be honest about the tradeoff this creates: Astro's islands ship literally zero JS on static routes, while Next.js App Router with React Server Components ships a small runtime even on server-rendered pages. "Minimal JS" is therefore the achievable target here, not "zero" — which makes the explicit perf budget below load-bearing rather than decorative.

### Tasks
- [ ] Next.js App Router project with design tokens visually consistent with authkestra.com (light + dark)
- [ ] Explainer/marketing routes as server components with no unnecessary client boundaries
- [ ] Playground route isolated so its client JS doesn't leak into explainer routes
- [ ] Audit the client bundle per route and keep `use client` boundaries as deep as possible
- [ ] Consistent header/footer linking back to docs and the GitHub repo

### Acceptance
Explainer routes ship only the framework's baseline runtime with no page-specific client JS; the playground route is the only place client-side interactivity is loaded.

#### Toggle panel component (renders from ScenarioSpec data)

`area:web` `type:feature`

### Tasks
- [ ] Render controls from the backend's scenario spec, not hardcoded markup
- [ ] Support boolean toggles and provider-select controls
- [ ] Optimistic UI with rollback on API failure
- [ ] Keyboard operable, correct ARIA roles and labels

### Acceptance
Adding a scenario server-side makes its control appear with no frontend change.

#### Diff viewer component

`area:web` `type:feature`

### Tasks
- [ ] Render the structured diff (added/removed/changed) legibly, not as raw JSON
- [ ] Show both the config change and its real-world consequence (routes, requirements, crate features)
- [ ] Handle wide content without breaking page layout (own scroll container)
- [ ] Readable in light and dark

### Acceptance
A visitor can tell what changed and why it matters without reading Rust.

#### Flow tester components for each scenario

`area:web` `type:feature`

The "then test it" half of the promise. Each scenario needs its own interaction surface.

### Tasks
- [ ] Passkey ceremony trigger + status, with the capability-detection fallback wired in
- [ ] TOTP QR display and code entry
- [ ] OAuth provider buttons and post-redirect return handling
- [ ] Resource-server tester: issue a token, call the protected route with it and without it
- [ ] Uniform result panel (what was sent, what came back, what it proves)

### Not in scope
Mounting the captcha widget, which moved to **Bot-protection scenario (Turnstile, hCaptcha, reCAPTCHA)** (P2). It is the only one of these surfaces that cannot be finished from this repository alone, and listing it here as a peer of the other four hid that. It now sits beside the dependency that gates it.

### Acceptance
Every v0 scenario that can be exercised from this repository is fully exercisable from the browser, with clear success and failure feedback. The result panel is a shared convention rather than a shared component: each panel shows what was sent, what came back and what it proves, in the same shape.

#### Set a performance budget and enforce it in CI

`area:web` `area:infra` `type:test`

"Optimal performance from day one" needs to be measured, or it silently degrades.

### Tasks
- [ ] Agree explicit budgets (LCP, CLS, INP, total JS on explainer vs playground routes)
- [ ] Lighthouse CI (or equivalent) gate on PRs
- [ ] Verify caching headers for static vs dynamic routes

### Acceptance
A regression past budget fails CI rather than being noticed after launch.

#### Accessibility pass on the auth flows

`area:web` `type:test`

Auth flows are exactly where a11y failures lock people out.

### Tasks
- [ ] Keyboard-only walkthrough of the full toggle → diff → test loop
- [ ] Screen-reader labels for ceremony states and async results (live regions for "waiting for your authenticator")
- [ ] Focus management across OAuth redirect returns
- [ ] Colour contrast in both themes

### Acceptance
The complete loop is operable keyboard-only and announces state changes to a screen reader.

#### Restructure the playground as a 3-step wizard

`area:web` `type:feature`

The playground is a wizard rather than a single panel, because the visitor's journey has three distinct questions: what do I want, what does it feel like, and how do I get it.

1. **Choose sign-in methods** — any combination (Google *and* GitHub *and* passkeys *and* TOTP). OAuth is therefore `SelectMany`, not `SelectOne`: two providers is not twice one provider's configuration.
2. **A real sign-in page** — assembled from what was chosen, beside the flow log, so the visitor sees both the surface and the machinery.
3. **Download** — the generated project (P4).

### Tasks
- [x] Step indicator with back-navigation to completed steps only
- [x] Step 1 renders methods generically from `ScenarioSpec`, including `SelectMany`
- [x] Step 2 assembles a genuine-looking sign-in page from the chosen methods
- [x] OAuth started as a top-level navigation, with the return read from the query string and cleared via `history.replaceState`
- [x] Placeholder scenarios removed from the shipped registry — the wizard lists whatever is registered, so a leftover fixture became product
- [ ] Step 3 wired to the generator once P4 lands

### Acceptance
A visitor picks several methods, sees a sign-in page built from exactly those, and can reach the download step.

#### Visitor-facing flow log

`area:api` `area:web` `type:feature`

The thing a playground can show that documentation cannot: what the engine actually did, in order, for *this* attempt. A challenge issued and held server-side, a code checked against the stored secret, a signature verified and a counter advanced 4 → 5.

Written for the visitor, not for us. Server-side `tracing` keeps the detail that would only confuse someone learning the flow.

### Tasks
- [x] `GET /api/session/events` returning `FlowEvent[]`, oldest first
- [x] `level` separates outcomes that look alike: a wrong code is `rejected`, not `failed`. A playground that renders an expected refusal as an error teaches the wrong thing.
- [x] `facts` for values worth showing verbatim (algorithm, period, signature counter) — never secrets, asserted by test
- [x] Capped per session and expiring with it, so nothing has to clean it up
- [x] Recording is infallible for the caller — a flow must never fail because its narration could not be written
- [x] `append_capped`/`list` in the store rather than read-modify-write, which would drop concurrent events
- [ ] Narrate the OAuth flow too, once a real round trip is possible (#7)

### The panel was removed; the record was not
The `FlowLog` component is gone from the sign-in step, and the space went to the resource-server panel (P2, the JWKS rebuild).

Everything ticked above still stands: `GET /api/session/events`, the levels, the facts, the cap, the expiry, the infallible recording, and the test that no secret reaches it. What was removed is the *reader*.

Two reasons, and the second settles it. The panel was expected to reflect the framework's own logs and did not — and it cannot usefully: authkestra emits roughly 34 `debug!` events to 6 `info!`, so an honest INFO mirror is nearly empty, while DEBUG would mean streaming unaudited log sites from a dependency to untrusted viewers. This log is safe *by construction* precisely because it is curated, and piping raw tracing through it would trade that away for content nobody can vet. Server-side `tracing` is where that detail belongs, and it now actually reaches the logs — the deployment had `authkestra=info`, which suppressed nearly all of it.

What the panel was reaching for is served better in place: each scenario's panel shows the request it made and what came back, next to the thing that produced it.

A reader may well come back for the OP-server page (P6), where a two-party flow genuinely benefits from a sequence. The endpoint is deliberately still there for it.

### Acceptance
A visitor completes and fails a flow, and the log reads as a sequence they can learn from. *Met by the API; the browser-side reader was withdrawn — see above.*

#### Dark theme, matching the framework's site

`area:web` `type:chore`

The framework's own site is dark; the playground should not look like a different product.

### Tasks
- [ ] Dark palette applied across the wizard, panels, diff viewer and flow log
- [ ] Contrast checked against WCAG AA for body text and the flow log's level colours — amber-on-dark and green-on-dark are the easy ones to get wrong
- [ ] Focus rings still visible against the dark background
- [ ] `color-scheme: dark` set so form controls and scrollbars follow

### Acceptance
The playground reads as part of the same product as the framework's site, and text and state colours stay legible.

---

## P4 — Downloadable starter kit

**Goal.** The playground's configuration becomes a real, compiling Cargo project the visitor can download and run — the bridge from demo to the v0-for-Rust wizard idea.

**Exit criteria.** Downloading a kit for any supported toggle combination yields a project that compiles and runs locally, whose behavior matches what was just demoed, verified by a CI compile matrix.

### Issues

#### Design the starter-kit template model (config → files + features)

`area:starter-kit` `type:feature`

The download turns the visitor's `DemoConfig` into a real Cargo project. This issue is the design, before any templating code.

Ground it in the framework's own examples — `crates/authkestra/examples/` already contains 14 Axum scenarios (`axum_basic_setup`, `axum_oauth2_github`, `axum_oauth_stateless`, `axum_oidc_google`, `axum_mfa_server`, `axum_op_server_sqlx`, …) which are the closest thing to templates that exists today and are CI-compiled upstream.

### Tasks
- [ ] Decide the mechanism (cargo-generate-style parameterised tree, or bespoke assembly)
- [ ] Map each scenario to the file fragments and Cargo features it contributes
- [ ] Decide the composition rules when several scenarios are on at once
- [ ] Pin versions from `Cargo.toml` / `workspace.package.version`, never from README prose (the upstream README is currently stale at 0.7 vs an actual 0.8.0)
- [ ] Decide the framework target for v0: Axum only (Actix is second-tier upstream — no macro parity, several examples have no Actix counterpart)

### Acceptance
A written spec that maps any valid toggle combination to a definite file tree and feature set.

#### Build the base starter-kit template

`area:starter-kit` `type:feature`

The always-present skeleton every generated project starts from.

### Tasks
- [ ] Axum service with the engine wired via the typestate builder
- [ ] Correct direct dependencies and feature flags (per the P0 dependency decision — not via the facade)
- [ ] Chosen TLS backend, with a note on the musl/`rustls-no-provider` caveat
- [ ] Store wiring: `SqlxCredentialStore` + SQLite by default; session store per the P0 decision
- [ ] `tracing` set up, matching upstream conventions
- [ ] Sensible `.gitignore`, dual license, and a `justfile`/`Makefile` for the common commands

### Acceptance
The base template compiles and runs with `cargo run` on a clean machine and serves a working endpoint.

#### Implement feature-gated template fragments per scenario

`area:starter-kit` `type:feature`

### Tasks
- [ ] Passkeys fragment (engine `webauthn`, credential store, both ceremonies, RP-ID config)
- [ ] TOTP fragment (engine `totp`, provisioning + verification)
- [ ] OAuth fragment (`authkestra-providers` with the selected provider features, callback routes, encrypted-cookie state)
- [ ] Bot-protection fragment (engine `captcha`, selected provider, verification placement)
- [ ] Fragments compose cleanly when multiple are selected (no duplicate routes, no conflicting state)

### Acceptance
Every fragment, alone and in combination, produces a compiling project.

#### Generate a tailored README and .env.example per download

`area:starter-kit` `area:docs` `type:feature`

A generated project whose README describes features you didn't pick is worse than no README.

### Tasks
- [ ] README generated from the selected config: what's included, how to run, what to configure
- [ ] `.env.example` listing exactly the required secrets for the chosen providers (and nothing else)
- [ ] Links back to the relevant authkestra docs pages and the upstream example each fragment derives from
- [ ] Explicit note on which credentials the developer must register themselves

### Acceptance
A developer can go from unzip to a running, correctly configured service using only the generated README.

#### Add the zip download endpoint

`area:starter-kit` `area:api` `type:feature`

### Tasks
- [ ] `GET /starter-kit` generating the project from the current session config
- [ ] Stream the archive rather than buffering large payloads
- [ ] Rate-limit generation (it's the most expensive endpoint on the service)
- [ ] Deterministic archive naming that reflects the selection
- [ ] Never include secrets — `.env.example` only, with placeholders

### Acceptance
The download matches the visitor's current configuration and contains no credentials.

#### CI compile matrix for generated projects

`area:starter-kit` `type:test`

**The starter kit's credibility rests entirely on this.** A generated project that doesn't compile is worse than no starter kit at all.

The full combinatorial space (passkeys × TOTP × 3 OAuth providers × 3 captcha providers) is too large to build exhaustively on every push, so the matrix needs a deliberate strategy.

### Tasks
- [ ] Define a representative combination set (all-off, each-alone, all-on, plus one per provider)
- [ ] CI job generating each combination and running `cargo check` (or `build`) on it
- [ ] Nightly/scheduled job covering a wider matrix than the per-PR set
- [ ] Fail loudly and specifically: report which combination broke
- [ ] Pin how the matrix reacts to a new upstream authkestra release

### Acceptance
No generated combination in the representative set can regress to non-compiling without CI failing.

#### Parity test: generated project behaves like the playground

`area:starter-kit` `type:test`

The implicit promise of the download is "this is what you just used". If they diverge, the demo becomes a lie.

### Tasks
- [ ] For each scenario, run the same flow assertions against a generated project and against the playground backend
- [ ] Cover route shapes, required inputs, and success/failure responses
- [ ] Document any deliberate differences (demo-only ephemerality, seeded users, etc.) in the generated README

### Acceptance
Each scenario passes the same behavioral assertions in both places, or the difference is explicitly documented.

#### Emit passkey and TOTP HTTP endpoints in the generated project

`area:starter-kit` `area:api` `type:feature`

The generator wires passkeys and TOTP into the engine builder but emits no HTTP surface for them, so a downloaded project can verify a credential and has no way to receive one. The visitor has to write the ceremony handlers themselves, which is the hardest part and the part the playground already solved.

This is a framework gap the kit can paper over: the adapters wire only three routes — `/auth/login/{provider}`, `/auth/callback/{provider}`, `/auth/logout` — all browser redirects. Passkeys and TOTP have no wired endpoints at all, so every application writes its own. The playground's own handlers are the reference.

### Tasks
- [ ] `KitFragment::routes`/`handlers` populated for passkeys: registration start/finish, authentication start/finish
- [ ] Same for TOTP: provision (returning the `otpauth://` URI) and verify
- [ ] Ceremony state stored the way the fragment's chosen store implies, not invented per handler
- [ ] Handlers return the same JSON shapes the playground uses, so the two cannot drift
- [ ] Extend the #32 compile matrix to assert the routes are present, not just that it builds

### Acceptance
A downloaded project with passkeys or TOTP selected can complete a full enrolment and verification against its own endpoints, with no handler written by hand.

### Note
Prerequisite for the OpenAPI/TS-client issue: there is nothing to describe or call until these exist.

#### Opt-in OpenAPI spec and typed TS client in the generated project

`area:starter-kit` `area:docs` `type:feature`

A downloaded project hands you a Rust backend and leaves the frontend entirely to you — hand-written `fetch` calls against endpoints whose shapes you have to read out of the source.

Deliberately scoped to this repository rather than the framework. authkestra's own wired surface is three browser redirects with no JSON shapes worth generating, and where it does have a JSON API (the OP) OIDC discovery already describes it. The shapes worth typing are the ceremony endpoints the *generator* emits, so the generator is the right place to describe them.

**Opt-in, not default.** A visitor who wants a Rust service should not be handed a TypeScript toolchain and an extra dependency they did not ask for.

### Tasks
- [ ] `utoipa` annotations on the generated ceremony handlers, behind a generated Cargo feature so the dependency is absent when unused
- [ ] Serve the spec from the generated project, and say where in the README
- [ ] Emit a small typed TS client for the passkey and TOTP endpoints — hand-written template, not a generator dependency
- [ ] The client covers the browser half honestly: `navigator.credentials` calls need base64url encode/decode around the JSON, which is where most passkey integrations go wrong
- [ ] Surface the choice on step 3 (download) rather than step 1: it is a property of the kit, not an authentication method, and putting it among the login toggles would misrepresent both
- [ ] Compile matrix covers the opt-in on and off

### Acceptance
A visitor who opts in gets a project serving its own OpenAPI spec and a TS client that completes a passkey ceremony without hand-written encoding. A visitor who does not opt in gets no `utoipa` in their dependency tree.

### Depends on
The ceremony endpoints issue — there is nothing to describe until those exist.

#### Ask for a GitHub star on download, without gating on it

`area:web` `type:feature`

Originally scoped as a gate: prove you starred `marcjazz/authkestra` before downloading.

**Decided against gating.** A star gate is trivially bypassed (star, download, unstar), puts a friction point on the critical path, and a minority of developers react badly to it. The download stays open; the ask stays.

### Tasks
- [ ] Star button / link on the download step, clearly optional
- [ ] Never a condition of the download, and never presented as one
- [ ] Optional: if the visitor already signed in with GitHub for the OAuth scenario, use that token to show whether they have starred it — as a nicety, not a check

### Acceptance
The download works for everyone; the ask is visible and honest.

#### Emit deploy-to-cloud manifests in the generated project

`area:starter-kit` `type:feature`

The generated project compiles and runs locally, and then stops. Getting it onto a host is left entirely to the visitor, which is the widest gap between "I downloaded it" and "it is serving traffic".

Manifests are driven by the same `Plan` that writes `.env.example`, so the env vars a host is told to set are the ones the code actually reads rather than a list that drifts.

### Tasks
- [ ] `Dockerfile` + `.dockerignore`, emitted by default — useful on every host and costs nothing
- [ ] `render.yaml`, `fly.toml` and `railway.json` as opt-ins
- [ ] Secrets declared as required-but-unset on every host; the archive must never carry a filled-in secret
- [ ] The port a manifest exposes matches what the generated `main.rs` actually binds
- [ ] README section with real next steps per host, and deploy buttons only where the host genuinely supports deploy-from-repo

### Acceptance
A generated project with a host selected deploys from its manifest without hand-editing anything but the secrets.

#### Push the generated project to the visitor's GitHub repo

`area:starter-kit` `type:feature`

A zip is fine; "create this as a repo in my account" is the v0-style magic moment. It is also what makes the deploy-to-cloud manifests useful: every one-click deploy button a host offers takes a repository URL, so without a repo the buttons have nothing to point at.

### Tasks
- [ ] Decide GitHub App vs OAuth app and the minimum scope, and record it as an ADR
- [ ] A separate OAuth app from the sign-in scenario, so the identity demo is never the thing asking for write access to someone's repositories
- [ ] Push as a single commit via the Git Data API, not a commit per file
- [ ] Handle failure modes distinctly (name collisions, revoked access, missing scope, rate limits) — a generic 500 for any of these is a bug
- [ ] Token stays server-side, session-scoped, short-lived, and never logged
- [ ] Keep the zip path as the no-auth fallback

### Acceptance
A visitor with a GitHub account gets a public repo holding the same project the zip would have given them, in one commit; a visitor without one still gets the zip.

---

## P5 — Launch hardening

**Goal.** Make the public surface safe, affordable, and measurable, then announce it.

**Exit criteria.** Security review closed, abuse/load test passed with cost guardrails verified, conversion instrumentation live, launch content published.

### Issues

#### Security review of the public demo surface

`type:security` `area:api`

A public playground for an auth framework is a uniquely embarrassing place to get owned.

### Tasks
- [ ] Cookie flags (HttpOnly, Secure, SameSite) and session-id entropy
- [ ] CORS policy between the Vercel frontend and the Fly.io API
- [ ] Secret handling review: nothing logged, nothing in the archive, nothing in client bundles beyond public site keys
- [ ] Verify session isolation cannot be crossed by manipulating the session cookie
- [ ] Confirm expired-session credential cleanup actually removes rows
- [ ] Review error responses for information leakage

### Acceptance
Review completed with findings either fixed or explicitly accepted and recorded.

#### Abuse and load test with cost guardrails verified

`type:test` `area:infra`

### Tasks
- [ ] Load test normal interactive traffic plus a scripted-abuse profile
- [ ] Confirm rate limits hold and the service degrades rather than falls over
- [ ] Verify billing alerts actually fire (test them, don't assume)
- [ ] Confirm the kill switch works under load
- [ ] Sanity-check third-party quota consumption (OAuth apps, captcha siteverify) under the abuse profile

### Acceptance
A hostile traffic profile produces throttling and alerts, not a surprise invoice or an outage.

#### Instrument conversion and usage

`area:web` `type:feature`

Without this there's no way to know whether the playground actually drives adoption, which is the entire point.

### Tasks
- [ ] Decide the metric that means success (repo stars, starter-kit downloads, docs click-through, crates.io pulls)
- [ ] Privacy-respecting analytics (no third-party surveillance on an auth demo — it undercuts the message)
- [ ] Track which scenarios get toggled, which flows complete, where visitors drop
- [ ] Track starter-kit downloads by configuration — this doubles as product research for the wizard idea

### Acceptance
A weekly view of what visitors try, what they complete, and what they download.

#### Write launch content and announce

`area:docs` `type:chore`

### Tasks
- [ ] Docs page on authkestra.com pointing at the playground (the site is Astro/Starlight, docs aren't CI-checked — link to real examples rather than inventing snippets)
- [ ] Launch post explaining what it is and the Rust-adoption angle
- [ ] Prepare for HN / r/rust: expect scrutiny of the generated code specifically
- [ ] Confirm the kill switch, rate limits and alerts are live before posting
- [ ] Have a plan for the first wave of feedback and issue triage

### Acceptance
Playground announced with working links, live guardrails, and a triage plan.

---

## P6 — Post-launch / wizard path

**Goal.** Backlog: what turns the playground into the broader 'v0 for Rust' scaffolder, plus cratestack integration.

**Exit criteria.** Not scoped for v0. Revisit once P5 ships and there is real usage data.

### Issues

#### Be your own identity provider: an OP server page

`area:scenario` `area:api` `type:feature`

`authkestra-op` ships and the playground does not mention it. Running your own OpenID Provider is the framework's most differentiated capability — most auth libraries let you *consume* an identity provider, not *be* one — and a playground that only shows passkeys, TOTP, OAuth and token validation undersells it by omission.

Held out of the resource-server work (#47) deliberately, and out of P2 for the same reason: **it does not fit the wizard.** Every other scenario is a toggle whose effect a visitor can see in one page. An OP is only meaningful once something authenticates *against* it, so demonstrating it needs a second application — a client — and that is a page of its own rather than a checkbox in step 1.

P6 rather than P2 because of that shape and that size, not because it matters less.

### What it would take
- [ ] A dedicated route, outside the three-step wizard, with its own explanation of what an OP is and why you would run one
- [ ] A demo client the visitor completes a real authorization-code flow against — the hard part, and the thing that decides whether this is worth doing
- [ ] Discovery and JWKS served and shown, since they are the machine-readable contract and the part integrators most often get wrong
- [ ] Mandatory PKCE, and loopback redirect URIs per RFC 8252 §7.3
- [ ] The failure modes surfaced the way #47 surfaces token failures, rather than a flat error
- [ ] Persistence via `OpStore`, with the decision about what the playground stores per visitor written down
- [ ] Starter-kit fragment, so a download can run an OP too

### Acceptance
A visitor completes an authorization-code flow against an OP the playground is running, sees the discovery document and the tokens it issued, and can download a project that does the same.

### Worth deciding first
Whether this belongs in the playground at all, or as a separate example application. It is a large amount of surface for one page, and the upstream repository already has four runnable `*_op_server*` examples. The case for building it here is that examples are read while a live flow is *used* — but that case should be made explicitly before the work starts, not assumed.

#### Backlog: reserve a cratestack scenario slot

`area:scenario` `type:feature`

Deliberately out of v0. When cratestack is ready, it should slot into the existing `ScenarioSpec` registry and get its own playground section rather than needing architectural change.

### Tasks
- [ ] Confirm the scenario registry can host it without refactoring
- [ ] Decide whether it earns its own nav section or sits alongside the auth scenarios
- [ ] Extend the starter-kit generator with cratestack fragments

#### Backlog: SeaORM and Diesel starter-kit variants

`area:starter-kit` `type:feature`

Upstream ships `authkestra-example-seaorm` and `authkestra-example-diesel` as compiled, conformance-tested example crates (not published libraries) — SeaORM is SQLite-only with no FKs; Diesel is sync via `spawn_blocking` with an r2d2 pool. Both pass the same `authkestra-store-testsuite`.

Offering "pick your data layer" in the generator is a strong DX story, but it multiplies the compile matrix, so it waits until the sqlx path is proven.

### Tasks
- [ ] Evaluate the maintenance cost of three store paths in the matrix
- [ ] Decide whether these become generator options or documentation only

#### Backlog: ephemeral live-preview container per generated config

`area:starter-kit` `area:infra` `type:feature`

The real bridge to the wizard idea: rather than toggling a shared always-on instance, spin up a throwaway container running the visitor's exact generated project, then tear it down.

This is where Rust's compile times bite — a generated project is seconds-to-tens-of-seconds to build, not the sub-second hot reload that makes JS-oriented tools feel instant. Any serious attempt needs pre-warmed build containers and a shared dependency cache.

### Tasks
- [ ] Prototype build-time with a warm cargo cache to see whether the UX is viable at all
- [ ] Decide orchestration, per-session limits, and teardown guarantees
- [ ] Model the cost per preview before committing

#### Contingency: a reCAPTCHA Enterprise path if the legacy secret key goes away

`area:scenario` `area:api` `type:chore`

Not a problem today. `RECAPTCHA_SITE_KEY` / `RECAPTCHA_SECRET_KEY` are set on the deployment with a legacy secret key, the classic `siteverify` endpoint accepts it, and the bot-protection scenario verifies reCAPTCHA end to end. This issue exists so that if that stops being true, the reasoning is already written down rather than rediscovered.

### The shape of the risk
`authkestra-engine`'s `CaptchaVerifier` verifies reCAPTCHA by posting `secret` and `response` as a form to `www.google.com/recaptcha/api/siteverify`. That is the only protocol the crate speaks, for all three providers.

Google has moved reCAPTCHA's console into Google Cloud and steers new keys towards **reCAPTCHA Enterprise**, which verifies through an assessment call to `recaptchaenterprise.googleapis.com/v1/projects/{project}/assessments`: a different endpoint, a Cloud project plus an API key rather than a shared secret, a JSON request body, and a response carrying `tokenProperties.valid` and a risk score rather than a bare `success` flag. It is a different protocol, not a different credential — so no amount of configuration bridges it.

Legacy secret keys still work, which is why nothing is broken. The exposure is that the playground's reCAPTCHA leg depends on a path Google describes as legacy, and its withdrawal would be a third party's decision on a third party's timetable.

### What breaking looks like
`verify` returns `Err`, the scenario reports `verified: false` with the provider's message, and the flow log says the check failed. It fails closed, so nothing becomes insecure — but reCAPTCHA stops being demonstrable, and the failure reads like a bad key rather than like a withdrawn protocol.

### The cheap mitigation, already in place
Nothing needs to be built to survive this. Unsetting `RECAPTCHA_SITE_KEY` / `RECAPTCHA_SECRET_KEY` removes reCAPTCHA from the control entirely: the credential-gated control only ever offers providers the deployment holds both halves for, and the scenario keeps working with Turnstile and hCaptcha, both of which still verify the classic way. That is a deployment change, not a code change.

The generated starter kit already says this — the reCAPTCHA setup step names the Enterprise split and points a reader at Turnstile or hCaptcha if their Cloud project will only issue Enterprise credentials.

### If it is ever worth building
A `CaptchaProvider::ReCaptchaEnterprise` variant **upstream in `authkestra-engine`**, carrying a project id and an API key, and reading the assessment response's validity and score.

Explicitly not in scope for this repository: reimplementing the assessment call in the playground. The scenario exists to demonstrate the framework's captcha feature, so hand-rolling the call here would demonstrate the playground instead, and the starter-kit fragment would generate code that does not match what it claims. If Enterprise is worth supporting it belongs in the framework, and this issue should become a pointer to the upstream one.

### Trigger to act
Any of: Google announcing a withdrawal date for legacy `siteverify`; the console ceasing to issue legacy secret keys for new sites; or the deployment's own reCAPTCHA verification starting to fail with a valid-looking key.

### Not blocked
Nothing is waiting on anyone. This is a watch item with a known escape hatch, deliberately left unlabelled `blocked:external` so that label keeps meaning work that genuinely cannot proceed.

#### Research: catalog cloud/PaaS provider revenue-share and kickback programs for templates

`area:monetization` `area:starter-kit` `type:research`

Railway runs a confirmed, real program — template creators earn 15% of the hosting costs a deployer incurs (25% with active support engagement in the Template Queue), paid via Railway credits or Stripe Connect. Its **Technology Partner** tier pays out on *any* community template using your technology, not just ones you built, and gives you a branded ecosystem page inside Railway's own dashboard — which is the closest thing found so far to "people get it from the provider's UI" without owning any infrastructure or data.

That is one confirmed data point. The goal of this ticket is to find out how many others exist, since the deploy-to-cloud feature (P4) will emit manifests for more than one provider and each one is a separate potential revenue channel that costs nothing to pursue if it exists.

### Scope — two distinct mechanisms, don't conflate them
1. **Usage-kickback / creator-payout programs** (Railway's model): the *provider* pays *you* a cut of what *they* bill the end user for hosting. Requires no selling, no pricing, no customer relationship — this is the one that fits "don't own the data."
2. **Paid marketplace listings** (AWS/Azure/GCP Marketplace, DigitalOcean paid Add-Ons): *you* set a price, *you* collect from the customer, the platform takes a cut of your revenue. Only relevant once there's something to actually sell (the managed-service side, not the free playground/starter-kit).

Don't file a provider under mechanism 1 without confirming money actually flows from the provider to the creator — a free 1-click-app listing with no payout is not a kickback program, it's just distribution.

### Providers to check (starting list, not exhaustive — find others)
- Render (Blueprints / marketplace)
- Fly.io (now the actual P0 host per this roadmap — check first, since a same-provider program would be the easiest to activate)
- DigitalOcean (confirm whether the free 1-Click Apps program has any creator payout at all, distinct from the paid Add-On revenue share already confirmed)
- Vercel (integrations/marketplace program — relevant to `apps/web`, not the Rust backend)
- Cloudflare (partner/marketplace programs — even though Workers itself is ruled out for the backend, confirm whether a program exists at all)
- Netlify, Coolify, Northflank, Zeabur, Koyeb, Porter, Qovery
- Heroku (button.heroku.com deploy-button ecosystem — legacy but confirm current state)
- AWS Marketplace (3% seller fee already confirmed for paid SaaS listings — separately check AWS Activate / ISV Accelerate for anything resembling a kickback rather than a fee reduction)
- Azure Marketplace, Google Cloud Marketplace
- Supabase, Neon, Upstash (not compute hosts, but if the session-store decision lands on Postgres/Redis-as-a-service, check whether either runs a referral/partner-revenue program worth stacking alongside a Railway/Fly deploy)
- Replit (deploy templates / bounty programs)

### Deliverable
A written comparison (`docs/research/provider-revenue-share-programs.md` in this repo) covering, per provider: does a program exist at all; which mechanism (kickback vs marketplace fee); exact percentage/terms with a source link; application/verification process and expected turnaround; payout method and minimum thresholds; and any constraint that rules it out for authkestra specifically (e.g. WASM-only runtime, no persistent compute, requires a paid listing you're not ready to run). End with a ranked recommendation of which 2-3 to actually pursue first, given the P0 host is Fly.io and the deploy-to-cloud feature is still unbuilt.

### Acceptance
Every provider in the starting list is either confirmed-with-source or confirmed-absent (not left blank), plus at least three more providers found that weren't on the starting list, and a clear top-2-3 recommendation with reasoning tied to authkestra's actual constraints (no data custody, real compiled-binary hosting needed, Fly.io already the P0 host).

---

## Labels

| Label | Meaning |
|---|---|
| `area:api` | Playground Rust backend (apps/api) |
| `area:web` | Playground frontend (apps/web) |
| `area:starter-kit` | Downloadable starter-kit generator |
| `area:infra` | Hosting, CI/CD, DNS, credentials, cost controls |
| `area:scenario` | An individual auth capability demo |
| `area:docs` | Docs, launch content, README |
| `type:feature` | New capability |
| `type:chore` | Setup, wiring, maintenance |
| `type:test` | Tests and verification |
| `type:security` | Security or abuse-surface work |
| `blocked:external` | Waiting on a third party (provider registration, upstream) |
| `good-first-issue` | Self-contained, good entry point for a contributor |
| `type:research` | Investigation/spike — deliverable is a written finding, not code |
| `area:monetization` | Revenue, provider partnerships, marketplace/kickback programs |

## How this syncs to GitHub

`roadmap.json` is the source of truth. `sync_github_issues.py` reads it and creates the labels, milestones (one per phase) and issues, idempotently — re-running it will not create duplicates. Regenerate this document with `python3 gen_roadmap_md.py` after editing the JSON.
