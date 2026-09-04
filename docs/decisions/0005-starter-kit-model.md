# 0005 — The starter-kit model: config → files + features

**Status:** accepted (design; implementation is P4 #28–#33)
**Date:** 2026-09-04
**Roadmap:** P4 "Design the starter-kit template model"

## What this has to do

Turn a visitor's `DemoConfig` — any combination of passkeys, TOTP and OAuth
providers — into a Cargo project that **compiles and runs**. Anything less makes
the playground a liar: the diff promises a configuration, and the download is
where that promise is checked.

## Mechanism: assemble in Rust, don't template a tree

Rejected `cargo-generate`-style parameterised trees. Two reasons, and the second
is the decisive one:

1. The output is small — one `main.rs`, a `Cargo.toml`, a `README.md`, an
   `.env.example`. A template engine's cost is mostly in managing conditionals
   across many files, which we do not have.
2. **Composition in this framework is a linear builder chain**, not scattered
   conditionals. `session_store()` and `token_manager()` are the only calls that
   move the typestate; `provider()`, `with_totp()`, `with_webauthn()` and
   `with_auth_method()` all return `Self`. So "TOTP and passkeys and GitHub" is
   three lines appended to one chain — which is a list to concatenate, not a
   tree to template.

So: each scenario contributes a **fragment**, and the generator concatenates
fragments into files. Plain Rust, unit-testable on the emitted text, and the
output is compiled in CI (#32).

## The fragment

Extends the existing `Scenario` trait rather than living beside it:

```rust
pub struct KitFragment {
    /// `use` lines this scenario needs.
    imports: Vec<&'static str>,
    /// Lines appended to the `Authkestra::builder()` chain, in order.
    builder_calls: Vec<String>,
    /// Setup that must run before the builder (a `Webauthn` instance, say).
    prelude: Vec<String>,
    /// Routes to merge, and the handlers behind them.
    routes: Vec<Route>,
    /// Crates and features, and the environment it expects.
    crates: Vec<CrateRequirement>,
    env: Vec<EnvVar>,
    /// What to tell the reader in the generated README.
    notes: Vec<String>,
}
```

### Why on the `Scenario` trait

`Scenario::consequences()` **already** returns the crates, features and routes a
scenario implies — that is what the diff renders. Deriving the generated project
from a second, parallel description would let the two drift, and the failure mode
is the worst kind: the playground promises one dependency set and the download
ships another.

One source of truth, and a test asserts the generated `Cargo.toml`'s feature
list matches the diff's `crates` entry for the same config. That test is most of
#33's parity requirement.

## Per-scenario mapping

| Scenario | Crates + features | Builder | Env |
| --- | --- | --- | --- |
| passkeys | `authkestra-engine` `webauthn`, `sql-sqlite`; `webauthn-rs`; `sqlx` `sqlite` | `.with_webauthn(Arc::new(webauthn), store)` | `WEBAUTHN_ORIGIN`, `WEBAUTHN_RP_ID` |
| totp (only method) | `authkestra-engine` `totp`, `sql-sqlite`; `sqlx` `sqlite` | `.with_totp(store)` | — |
| totp (with another) | as above | `.with_mfa_method(TotpAuthMethod::new(store))` | — |
| oauth: each provider | `authkestra-providers` `<provider>`; `authkestra-engine` `session`, `token` | `.provider(OAuth2Flow::new(P::new(id, secret, uri)))` | `<P>_CLIENT_ID`, `<P>_CLIENT_SECRET`, `OAUTH_REDIRECT_BASE` |
| oauth: google | additionally `authkestra-oidc` | — | — |

Always emitted, whatever is selected: `authkestra-axum` with `macros`,
`session`, `token`; a TLS backend per `0001`; `axum`, `tokio`, `tower-cookies`.

## Composition rules

1. **Order is fixed**, not selection order: prelude → `session_store` →
   auth methods (passkeys, TOTP) → providers → `build()`. Deterministic output
   means a diffable download and a cacheable zip.
2. **TOTP changes role by company.** Alone, it is a first factor
   (`with_totp`). Alongside passkeys or OAuth it is registered as step-up
   (`with_mfa_method`), because the framework's own MFA example does exactly
   that and TOTP-as-sole-factor is a weaker design than the visitor probably
   intends. **The generated README states which role was chosen and how to
   change it** — a silent semantic difference would be worse than either
   default.
3. **Features union, never repeat.** Two providers produce one
   `authkestra-providers` entry with two features, mirroring how Cargo actually
   resolves additive features — and matching what the diff already shows.
4. **A credential store appears if any method needs one.** Passkeys and TOTP
   both require `CredentialStore`; it is emitted once and shared, as in
   `axum_mfa_server`.
5. **Nothing selected** still emits a compiling project: session-only, with a
   README explaining that it is the smallest useful `Engine`. An empty download
   that fails to build would be the worst possible first impression.

## Versions

Pinned from the framework's `[workspace.package] version` (`0.8.0`), never from
prose. The upstream README currently says `0.7` while the crates are `0.8.0`,
which is precisely why this is written down. The generator reads the version
from one constant, and a test asserts it matches what the playground itself
depends on — so the download can never advertise a version we do not build
against.

## Scope for v0

**Axum only.** Actix is second-tier upstream: no `macros` parity, and several
examples have no Actix counterpart. Shipping a broken Actix variant would cost
more than omitting it. The fragment model is framework-agnostic, so adding
Actix later is a second set of fragments rather than a redesign.

Also out of scope: a database beyond SQLite, and any user/account table. The
framework deliberately owns no user trait, so the generated project keeps that
boundary — the README says so explicitly rather than inventing a schema.

## File tree

```
authkestra-starter/
  Cargo.toml          pinned deps, unioned features
  src/main.rs         assembled builder chain + routes
  README.md           what was selected, how to run, what to set
  .env.example        exactly the variables this configuration reads
  .gitignore
```

Flat and single-crate on purpose: the reader should be able to hold the whole
thing in their head, and `cargo run` should be the only instruction.

## How this stays honest

- The generated project is **compiled in CI** across the meaningful toggle
  combinations (#32). A starter kit that does not build is worse than none.
- The generated `Cargo.toml` is asserted against the diff's own crate list, so
  the promise and the artefact cannot drift (#33).
- The pinned version is asserted against the playground's own dependency.
