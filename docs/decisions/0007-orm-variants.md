# 0007 — SeaORM and Diesel: documentation, not generator options

**Status:** accepted
**Date:** 2026-09-14
**Roadmap:** #39 "Backlog: SeaORM and Diesel starter-kit variants"

## Context

`0005` scoped the generator to SQLite via `sqlx` for v0 and named "a database
beyond SQLite" as deliberately out of scope. Upstream has since grown two more
store paths — `authkestra-example-seaorm` and `authkestra-example-diesel` —
both passing the same `authkestra-store-testsuite` as the `sqlx` path the
generator already emits. #39 asks two things: what the three-store matrix
would cost to maintain, and whether that cost buys generator options or just a
documentation link.

### What the three paths actually are

- **sqlx** (shipped): async, SQLite, the store the generator emits today via
  `SqlxCredentialStore<sqlx::Sqlite>` (`scenario/passkeys.rs`,
  `scenario/totp.rs`).
- **SeaORM** (upstream example): SQLite-only, **no foreign keys**.
- **Diesel** (upstream example): **sync**, run via `spawn_blocking`, pooled
  with r2d2.

These are not three configurations of one implementation, the way the three
captcha providers are (`scenario/captcha.rs`: "one implementation, three
configurations... the only thing that varies is which `siteverify` endpoint it
posts to"). SeaORM drops referential integrity the schema otherwise has, and
Diesel changes the *execution model* the generated `main.rs` and its
`Cargo.toml` features would need to express — a real async/sync fork, not a
swapped enum variant and secret name. Each would need its own fragment code
for both passkeys and TOTP, not a parameter on the existing one.

They are also, unlike `sqlx`, **not published libraries**. They are compiled,
conformance-tested example crates. The generator would be depending on — and
tracking breakage in — code upstream ships as a demonstration, not as an API
surface upstream has committed to keep stable. `sqlx` is a real dependency of
`authkestra-engine` itself; SeaORM and Diesel support is not.

### What crossing them would cost, quantified

Today's matrix (`kit/matrix.rs`):

- **Representative (pull requests): 14** combinations — `base`, `passkeys`,
  `totp`, `resource`, one per OAuth provider (3), one per captcha provider
  (3), `totp-passkeys`, `totp-captcha`, `all`, `all-extras`.
- **Exhaustive (nightly): 128** — the full product, `8 toggle subsets × 8
  provider subsets × 2 (captcha off/on)`, per `exhaustive()`'s own test
  (`assert_eq!(exhaustive().len(), 8 * 8 * 2)`).

A credential store is only instantiated when passkeys or TOTP (or both) is
selected — 6 of the 8 toggle subsets. Adding SeaORM and Diesel as a third
store dimension, crossed the same way captcha and OAuth are, multiplies just
those 6 subsets by 3: `(6 × 8 × 2 × 3) + (2 × 8 × 2) = 288 + 32 = 320`
nightly jobs — before counting a representative leg per store path per
store-using scenario.

**320 exceeds GitHub Actions' 256-job matrix ceiling outright.** This is not a
new concern invented for this decision: `matrix.rs` already declines to cross
all three *captcha* providers for exactly this reason ("crossing all three
would put the nightly matrix at GitHub's 256-job ceiling to prove the same
thing three times"), and captcha providers are cosmetic variants of one
fragment. Store paths are a structural fork of two fragments each. The same
ceiling this file already designs around would be hit by a strictly smaller,
strictly cheaper change than the one #39 proposes.

## Decision

**SeaORM and Diesel become documentation, not generator options.** The
starter kit continues to emit `sqlx`/SQLite only. The generated README gains a
note, alongside passkeys and TOTP's existing store-backed fragments, pointing
at `authkestra-example-seaorm` and `authkestra-example-diesel` upstream for a
reader who wants a different store — the same way `0005` already tells the
reader the framework owns no user table rather than inventing one.

Three reasons, in order of weight:

1. **They are example crates, not a dependency surface.** The generator's
   honesty rule (`0005`: "the generated `Cargo.toml`... asserted against the
   diff's own crate list, so the promise and the artefact cannot drift") only
   works against something upstream has committed to keep working. An example
   crate can be rewritten or dropped without a deprecation cycle; `sqlx`
   cannot, because `authkestra-engine` itself depends on it.
2. **The fragments would not compose the way the existing model assumes.**
   `0005`'s fragment model — imports, builder calls, crates, env, notes —
   is built for parameters that differ by enum variant. SeaORM's missing FKs
   and Diesel's sync-via-`spawn_blocking` model are differences in what the
   generated code *does*, which means two more full sets of passkeys and TOTP
   fragments to write and keep in sync with upstream's example crates, not
   two more match arms.
3. **The matrix cost is not proportional, it is prohibitive.** 320 nightly
   jobs is past the ceiling this repository already treats as a hard
   constraint, for three store paths versus the captcha scenario's three
   cosmetic variants, which stayed under it deliberately. Getting back under
   256 would mean *not* crossing store fully — building it representatively
   only — which quietly undermines the actual promise: that whichever store a
   visitor picks compiles in the composition they picked it in.

## Consequences

- No new `store` control, no new `ScenarioOption`s, no change to
  `matrix.rs`'s 14/128 split. This decision adds no CI cost.
- The generated README documents SeaORM and Diesel as an upstream path for a
  reader who outgrows SQLite, next to the existing "framework owns no user
  table" note from `0005` — same shape, same honesty rule: point at what
  exists rather than promise what is not built.
- This is revisited, not closed permanently: if SeaORM or Diesel graduate
  from example crate to a published, versioned store crate upstream — the
  same status `sqlx` already has — the "not a dependency surface" objection
  above disappears and only the matrix-size and fragment-duplication
  objections remain, which is a narrower question worth its own decision.
- Until then, a visitor who wants SeaORM or Diesel gets a working `sqlx`
  project plus a link, rather than a generator option that compiles for CI's
  three legs and silently drifts everywhere else.
