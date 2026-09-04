# 0002 — Demo session storage: Memory for v0

**Status:** SUPERSEDED by [`0004-stateless-service-on-redis.md`](0004-stateless-service-on-redis.md)

> Kept as the record of why Memory was chosen first, and of the storage
> constraints in the framework that are still true. The decision itself no
> longer holds: all state now lives in Redis with a TTL, and the process holds
> nothing durable.

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
| WebAuthn/TOTP credentials (arrives in P2) | `SqlxCredentialStore` + SQLite | No — container filesystems are ephemeral, see below |

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
made stable across restarts (set it with `fly secrets set`); without it a random per-process key is generated
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

## Credential storage on an ephemeral filesystem

Resolved by the move to a container platform (`0003-hosting-platform.md`): the
container filesystem is ephemeral, so P2's SQLite-backed passkey/TOTP
credentials **will not survive a redeploy**. That is consistent with the demo
sessions that own them — a visitor whose session is gone has no use for the
credentials it created — so it needs no separate mitigation, only for P2 to
expect it rather than be surprised by it.

If credentials ever need to outlive a deploy, attach a Fly volume or move the
credential store to Postgres via the `sql-postgres` feature.
