# 0003 — Backend hosting: Fly.io, not Shuttle

**Status:** accepted (v0)
**Date:** 2026-09-04
**Roadmap:** P0 "Set up Shuttle project and deploy-on-merge for apps/api"
**Supersedes:** the Shuttle assumption baked into the original roadmap

## Context

The roadmap assumed Shuttle as the backend host. That assumption no longer
holds. Checked on 2026-09-04:

- `shuttle-hq/shuttle` on GitHub is **archived**; last push 2026-01-09
- `shuttle-runtime` was last published to crates.io on **2025-09-11** — roughly
  a year stale
- `shuttle-hq/deploy-action` last saw a commit on 2025-01-23
- `console.shuttle.dev` now redirects to `shuttle.dev`

Deploying a public, credential-handling demo onto an abandoned platform is not a
defensible choice: no security updates to the runtime, no support, and a real
possibility of the platform disappearing.

A concrete symptom surfaced during the brief Shuttle integration: `cargo deny`
failed on **RUSTSEC-2026-0173** (`proc-macro-error2` unmaintained), which
reached the graph *only* through `shuttle-codegen`. Dropping Shuttle removed the
advisory rather than requiring us to carry an accepted exception.

## Decision

**Fly.io**, deploying a container built from `apps/api/Dockerfile`.

### Why not serverless

Cloud Run, Lambda and similar were ruled out on architecture, not price. The
playground keeps demo sessions **in process memory** and sweeps expiry with a
`tokio::interval` (see `0002-session-store.md`). Both assume a long-lived
process. Request-scoped compute that scales to zero would silently reset every
visitor's configuration between requests and stall the sweeper.

That constraint is encoded in `fly.toml`:

```toml
auto_stop_machines = false
min_machines_running = 1
```

**These are load-bearing, not cost tuning.** Turning on scale-to-zero, or
running more than one machine, breaks the session model — a second machine would
hand a visitor a different configuration depending on which one they hit. Revisit
only if the session store moves to Redis.

### Consequences

- The deployment artifact is a plain container running the ordinary
  `#[tokio::main]` binary. There is no platform-specific entrypoint, no runtime
  macro, and no vendor SDK in the dependency graph — so the next migration is a
  new `Dockerfile` target rather than a code change.
- Local, CI and production all run the same binary with the same env-var
  configuration.
- The `Dockerfile` installs `build-essential`, `cmake` and `perl` because
  `aws-lc-rs` compiles C and assembly (see `0001`). The TLS choice and the base
  image are coupled; `rustls-no-provider` would allow a smaller builder.
- `ca-certificates` is installed in the runtime stage. Without a trust store
  every outbound call to an identity provider fails at handshake.
- Secrets are set with `fly secrets set`, which is a straight substitute for the
  Shuttle secret store — the app still reads plain env vars.

## What a human still has to do

1. `fly launch --no-deploy` from the repo root (reuses the committed `fly.toml`)
2. `fly secrets set` for provider credentials and `ADMIN_TOKEN` / `OAUTH_STATE_KEY`
3. Add `FLY_API_TOKEN` (from `fly tokens create deploy`) as a GitHub repo secret
4. Point `ALLOWED_ORIGINS` at the Vercel domain, and the frontend's
   `NEXT_PUBLIC_API_BASE_URL` at the Fly hostname
