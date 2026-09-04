# 0004 — A stateless service, with all state in Redis

**Status:** accepted
**Date:** 2026-09-04
**Supersedes:** `0002-session-store.md` (Memory for v0)
**Amends:** `0003-hosting-platform.md` (which assumed a persistent process)

## Context

`0002` chose an in-memory session store, and `0003` chose Fly.io partly *because*
it runs a persistent process — the in-memory store and the `tokio::interval`
expiry sweep both required one.

Two things then happened:

1. The Fly trial ended. Fly suspended the machine, released its IPv6 address and
   refused further deploys. The symptom was confusing — TLS handshakes still
   succeeded, because Fly's edge kept serving the certificate, while nothing
   answered behind it.
2. Reviewing the options made the real cost of the design visible. The genuinely
   free tiers all either scale to zero or recycle instances, and the
   "persistent process" requirement was the only thing ruling them out. It was
   not a requirement of the *product*; it was a consequence of one storage
   choice.

## Decision

**Hold no durable state in the process.** Everything moves to Redis with a TTL:

| State | Key | TTL |
| --- | --- | --- |
| Demo session (visitor's config) | `session:{id}` | 12h, refreshed on write |
| WebAuthn challenge | `ceremony:{session}:{kind}` | 5 min, single-use |
| TOTP secret / passkey | `cred:{session}:{type}:{id}` | session TTL |

Consequences that matter:

- **The sweeper is gone.** Redis expiry replaces it. A background task is
  exactly the thing that silently stops working once the process is allowed to
  sleep, so removing it is a correctness improvement, not just a simplification.
- **The single-instance constraint is gone**, along with the `fly.toml` comments
  warning about it. Any instance can serve any visitor.
- **SQLite is gone.** It needed a writable filesystem, which a recycled
  container does not have. `sqlx` left the dependency graph with it.
- Local development needs no Redis: `REDIS_URL` unset falls back to an
  in-process store, logged loudly, and the test suite runs on it. The Redis
  backend is covered by the same contract tests, which skip unless `REDIS_URL`
  is set — CI supplies one as a service container so they actually run.

### TLS to a managed provider

`authkestra-engine` declares `redis` with only `tokio-comp`, so its own
`RedisStore` cannot speak `rediss://`. Rather than fork or patch it, this crate
enables `tokio-rustls-comp` and `tls-rustls-webpki-roots` on the same `redis`
crate — Cargo features are additive, so the single `redis` in the graph gains
TLS, and the engine's store benefits through `RedisStore::with_client`, which
accepts an already-open client.

### A credential-store bug this fixed

The framework's `SqlxCredentialStore` derives a credential's id from
`data["credential_id"]`/`data["id"]`, falling back to a random UUID — while
`update_credential` is called with the id the *browser* used. For a passkey
those never match, so the signature-counter update lands on zero rows, returns
`Ok`, and is silently lost.

`register_totp` also mints a fresh `credential_id` on every enrolment, so
enrolling twice stores two secrets, and verification reads whichever the store
returns first. A visitor who re-scanned a QR code would then be checked against
the secret they had just discarded, and every code would be rejected — which
reads as the framework being broken.

The Redis store files a session's TOTP secret under a fixed id (a session holds
at most one) and a passkey under its `cred_id`, and keeps a back-reference from
every id the framework might later use, so updates resolve. Both behaviours are
pinned by tests.

## What this does not solve

- Redis is now a hard dependency in deployment. A free managed tier is enough at
  demo scale, but the service is down if Redis is, so `state_from_env` connects
  and PINGs at boot rather than failing on a visitor's first request.
- Command quotas on free Redis tiers are a real limit. The design is frugal —
  one read per request, one write per change — but a busy launch day is worth
  watching.
