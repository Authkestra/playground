# 0002 — Demo session storage: Memory for v0

**Status:** accepted (v0)
**Date:** 2026-09-04
**Roadmap:** P0 "Decide the session store: Redis vs Memory"

## Context

The framework's generic `SqlKvStore` was **deleted** in the Epic #289 storage
rework, not deprecated. There is no "put the session blob in Postgres"
primitive any more. SQL storage is now scoped to two specific uses:

- `SqlxCredentialStore` (WebAuthn/TOTP credentials) — in `authkestra-engine`,
  behind `sql-sqlite` / `sql-postgres` / `sql-mysql`
- `SqlxOpStore` (OAuth/OIDC-provider tables) — the `authkestra-store-sqlx` crate

So generic session KV is **Memory or Redis**, and nothing else.

## Decision

**Memory for v0.** Two separate stores are in play and it is worth keeping them
distinct:

| What | Where | Survives redeploy? |
| --- | --- | --- |
| The playground's own `DemoSession` (config, TTL) | in-process `HashMap` (`apps/api/src/session.rs`) | No |
| authkestra's `SessionStore` for the per-visitor engine | `MemoryStore` | No |
| WebAuthn/TOTP credentials (arrives in P2) | `SqlxCredentialStore` + SQLite | Depends on Shuttle's filesystem — **unconfirmed**, see below |

Rationale: demo sessions have a 12h TTL and carry nothing a visitor would mourn.
Losing in-flight sessions on a redeploy is a minor annoyance, not data loss.
Memory costs nothing, adds no infra, and keeps v0 inside the cost guardrails.

This is a conscious choice, not a discovery in production — which is what the
roadmap asked for.

## Redeploy behaviour (what a visitor actually loses)

On every redeploy, **every visitor's configuration resets to defaults** and any
in-flight OAuth round-trip fails. The visitor sees the playground as if they had
arrived fresh. No error, no data loss, no manual recovery.

`OAUTH_STATE_KEY` exists so that the encrypted-cookie OAuth state key can be
made stable across restarts; without it a random per-process key is generated
and logged as a warning.

## Migration path to Redis

`SessionStore` is a trait, so the authkestra half is genuinely a one-line change
(`MemoryStore::default()` → a Redis store, plus the `redis` feature on
`authkestra-engine`). The playground's own `DemoSessionStore` would need a real
implementation behind the same method set — it is deliberately a small surface
(`create` / `get` / `update_config` / `reset` / `sweep`) for that reason.

Switch when either becomes true:
- more than one instance is served (Memory cannot scale past one), or
- losing configurations on deploy starts generating complaints.

## Open question for the human

Shuttle's filesystem persistence across redeploys is **not yet confirmed**. It
does not affect this decision (sessions are in memory either way) but it does
decide whether P2's SQLite-backed passkey/TOTP credentials survive a deploy.
Losing them is acceptable given the 12h TTL — but it should be confirmed rather
than assumed. Tracked in the P0 Shuttle issue.
