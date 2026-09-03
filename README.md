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
> capabilities already shipped in authkestra 0.8.0.

## Status

**v0 — core machinery.** The session/config/diff/safety layer that every
scenario depends on is built and tested. The user-visible auth scenarios
(passkeys, TOTP, OAuth, bot protection) are the next phase and are not
implemented yet; two placeholder scenarios exist to exercise both control
shapes. Nothing is deployed — see [Deployment](#deployment).

## Layout

```
apps/api/            Rust (Axum) backend, deploys to Fly.io
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
| `SESSION_TTL_HOURS` | `12` | Demo session lifetime |
| `DEMO_ENABLED` | `true` | Global kill switch for live flows |
| `DEMO_DISABLED_SCENARIOS` | — | Comma-separated scenario ids to disable |
| `ADMIN_TOKEN` | — | Enables `POST /admin/kill-switch`. **Unset means the endpoint is not mounted at all.** |
| `OAUTH_STATE_KEY` | — | ≥32 bytes; keeps encrypted OAuth state valid across restarts |
| `OAUTH_REDIRECT_BASE` | `http://localhost:8000` | Base for provider redirect URIs |
| `<PROVIDER>_CLIENT_ID` / `_SECRET` | — | `GITHUB_`, `GOOGLE_`, `DISCORD_` |

## What a visitor gets

Each visitor gets an isolated demo session (12h TTL, id in an HttpOnly cookie).
Toggles affect only that visitor. Two people using the site at once never see
each other's configuration.

**Sessions live in memory and do not survive a redeploy** — on deploy, every
visitor's configuration resets to defaults and the site behaves as if they had
just arrived. That is a deliberate v0 choice; see
[`docs/decisions/0002-session-store.md`](docs/decisions/0002-session-store.md).

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

## Safety

This is a public, unauthenticated surface that will eventually create real
WebAuthn credentials and call third-party providers, so it ships with a front
door:

- **Rate limiting** — per-IP token buckets, deliberately tighter on endpoints
  that hit third parties. Returns a JSON 429 the UI renders as a friendly note.
- **Kill switch** — `DEMO_ENABLED` plus per-scenario disable, flippable at
  runtime through `POST /admin/kill-switch` without a redeploy. With flows off
  the site degrades to explainer-only mode rather than looking broken.

## Deployment

Backend to **Fly.io** (container built from `apps/api/Dockerfile`), frontend to
**Vercel**, both on merge to `main`.

Shuttle was the original plan; it was dropped after the project turned out to be
abandoned — see
[`docs/decisions/0003-hosting-platform.md`](docs/decisions/0003-hosting-platform.md).

⚠️ **`fly.toml` pins `auto_stop_machines = false` and `min_machines_running = 1`
for architectural reasons, not cost.** Demo sessions live in process memory and
the expiry sweeper is a `tokio::interval`, so scale-to-zero would reset every
visitor's config, and a second machine would serve different configs depending
on which one a visitor reached. Don't relax these without moving the session
store to Redis.

Secrets go in with `fly secrets set`; the app reads plain env vars either way.
The Fly app, Vercel project, DNS and provider credentials still need creating —
see the open P0 issues for the exact steps and secret names.

## CI

Mirrors the framework's bar: `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test`, `cargo llvm-cov`, `cargo deny check`, plus frontend typecheck,
lint and build. The coverage gate starts at 50% (the framework enforces 84) and
should ratchet up.

## Licence

MIT OR Apache-2.0, matching the framework.
