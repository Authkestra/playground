# 0001 — authkestra dependency block and TLS backend

**Status:** accepted (v0)
**Date:** 2026-09-04
**Roadmap:** P0 "Pin the authkestra dependency strategy and TLS feature choice"

## Context

Two traps, both flagged in the roadmap, both confirmed by reading authkestra 0.8.0:

1. **The facade is not enough.** `authkestra` (the facade crate) exposes only
   `session`, `token`, `oidc`, `axum`, `actix`, `resource`, `github`, `google`
   and `discord`. It does **not** expose `webauthn`, `totp`, `captcha`, `op`,
   `devsig` or any store backend — those exist on `authkestra-engine`,
   `authkestra-axum` and `authkestra-store-sqlx`, and the facade pulls them in
   only as `[dev-dependencies]` for its own examples. Verified in
   `crates/authkestra/Cargo.toml` at 0.8.0.
2. **The TLS backend is a mandatory either/or**, forwarded through every crate
   that speaks HTTPS. Features are additive, so one stray dependency can drag
   `aws-lc-rs` back in even if the app asked for the pure-Rust path.

## Decision

### Depend on the sub-crates directly, never the facade

```toml
authkestra-engine = { version = "0.8.0", default-features = false, features = [
    "token", "session", "memory", "totp", "webauthn", "sql-sqlite", "captcha",
    "rustls-aws-lc-rs",
] }
authkestra-axum = { version = "0.8.0", default-features = false, features = [
    "macros", "session", "token", "captcha", "rustls-aws-lc-rs",
] }
authkestra-providers = { version = "0.8.0", default-features = false, features = [
    "github", "google", "discord", "rustls-aws-lc-rs",
] }
```

`default-features = false` is set on the **workspace** entry as well as the
member entry. Cargo ignores `default-features = false` on an inherited
dependency unless the workspace entry sets it too — this is the same trick the
framework's own workspace uses, and without it the TLS choice cannot be turned
off downstream.

Published on crates.io: `authkestra` and its sub-crates are all at `0.8.0`, so
this is a registry dependency, not a git one. `Cargo.lock` is committed.

### TLS: `rustls-aws-lc-rs` (the default)

Rejected `rustls-no-provider` for v0 because it requires the application to
install a `rustls::CryptoProvider` **before any HTTP client is constructed**, or
`reqwest` panics at construction. That is a sharp edge to hand a newcomer in a
generated starter kit — the whole point of the kit is that it runs. Shuttle
builds in a standard glibc image with a C toolchain, so `aws-lc-rs` compiling C
and assembly is not a problem there.

**Escape hatch**, documented for anyone who needs it (musl targets, or a
`cargo-deny` policy that bans `aws-lc-rs`): turn defaults off and install a
provider yourself, per the framework README.

### Verification

`cargo tree -i aws-lc-rs -e features` confirms the backend that actually lands
in the graph, reached consistently through all three crates:

```
aws-lc-rs v1.18.1
└── rustls v0.23.43
    └── hyper-rustls v0.27.9
        └── reqwest feature "__rustls-aws-lc-rs"
            └── reqwest feature "rustls"
                └── authkestra-engine feature "rustls-aws-lc-rs"
                    ├── authkestra-axum feature "rustls-aws-lc-rs"
                    └── authkestra-providers feature "rustls-aws-lc-rs"
```

Re-run that command after any dependency bump. If a crate appears that enables
the other backend, the graph will show both and the choice is no longer ours.

## Consequences

- The starter-kit generator (P4) emits **this block verbatim**, so a downloaded
  project matches what the playground itself compiles.
- Feature drift between playground and generated kit becomes a visible diff in
  one file rather than a mysterious runtime difference.
- `ring` also appears in the tree as a transitive of `rustls`' own defaults.
  That is expected and is not a second crypto backend in use.
