# 0006 — Pushing a generated project to the visitor's GitHub

**Status:** accepted
**Date:** 2026-09-12
**Roadmap:** #40 "push generated project to the visitor's GitHub repo"

## Context

The starter kit already exists as a zip download (`0005`). #40 asks for a
second delivery path: create a repository on the visitor's own GitHub account
and push the same generated project to it, so the next step after "I like this
configuration" is `git clone` rather than "unzip, then `git init`".

That needs an OAuth round trip with GitHub, a token good for at least a
repository-create and a handful of Git Data API calls, and — because a public
demo means anyone can try this — a design where the worst outcome of getting
something wrong is "a visitor's push fails to be created", not "a visitor's
GitHub account is exposed to more than they agreed to."

## Decision

### A second, separate OAuth App — never the sign-in one

The sign-in scenario (`scenario::oauth`) already registers GitHub as an
identity provider, with its own `GITHUB_CLIENT_ID`/`GITHUB_CLIENT_SECRET`. This
feature does **not** reuse it. It reads `GITHUB_KIT_CLIENT_ID` /
`GITHUB_KIT_CLIENT_SECRET` instead, and registers a second, independent OAuth
App at GitHub.

The reason is not mechanical, it is what the two scenarios are *for*. Sign-in
exists to demonstrate identity, and the honest way to demonstrate an identity
flow is to ask for the narrowest scope that proves who someone is — which for
GitHub sign-in is no scope at all beyond the public profile. Pushing to a
repository needs `public_repo`, a scope with nothing to do with identity.
Folding the two together would mean every visitor who just wanted to try
"sign in with GitHub" silently authorized repo-write access on the same
click — which is exactly the kind of scope creep a playground that *teaches*
OAuth should model as abnormal, not ship as its own default. A visitor
inspecting the two consent screens side by side is supposed to see a real
difference, because there is one.

Both credential pairs degrade the same way when absent: the affected route
reports itself unavailable (`github_push_not_configured`, or the equivalent
`InvalidValue` the sign-in login route already returns) rather than sending
the visitor to a provider that can only reject them. Registering either app is
an operator's job, same as today.

### GitHub App vs. OAuth App: OAuth App, deliberately

A GitHub App (installed per-repository or per-account, with fine-grained
permissions and short-lived installation tokens) is the modern, more
capable-scoped option, and was considered. It was rejected for v0 for a
concrete reason: a GitHub App must be *installed* on an account or
organization before it can act, which is a second, separate consent step
(`https://github.com/apps/<name>/installations/new`) on top of the OAuth
authorization — and a demo visitor abandoning at the extra step is a real
cost this playground has consistently optimized against elsewhere (see the
`oauth` scenario's own "offer only configured providers" reasoning). An OAuth
App's single consent screen, scoped to `public_repo`, gets a first-time
visitor from "I like this configuration" to a pushed repository in one
provider round trip. If a later iteration wants org-repo support or
fine-grained, revocable-per-repo tokens, a GitHub App is the right vehicle for
that and can be added beside this without disturbing it.

### `public_repo` is the ceiling — not `repo`

The narrowest scope that can create and write to a repository under the
connected account is `public_repo`. `repo` additionally grants read/write on
**every private repository the account can see**, which this feature has no
use for: pushed repositories are created public, deliberately (see below), so
the wider scope would be pure unused blast radius sitting on a token this
service holds, even briefly.

**Private-repo support is out of scope for v0, on purpose**, not an oversight.
If it is wanted later, it should be its own decision — weighing whether
`repo`'s account-wide reach is acceptable for a public playground token store,
or whether it waits for the GitHub App path above, which can grant
per-repository access without the account-wide scope at all. Widening the
scope quietly, in the same change that shipped `public_repo`, would be the
mistake this section exists to head off.

### The token: session-scoped, server-side, fifteen minutes

The exchanged access token is stored in the shared key-value store — the same
backing store as demo sessions and scenario credentials (`0004`) — keyed by
the visitor's demo session id, and is:

- **Never sent to the browser.** It travels callback → store → push handler
  and nowhere else; no response body, redirect query string, or cookie ever
  carries it.
- **TTL'd at fifteen minutes**, independent of the twelve-hour session TTL.
  The token only has to survive the time between clicking *Connect* and
  clicking *Push* — one visitor, one button, in one sitting. Fifteen minutes
  covers someone who pauses to type a repository name; it does not cover
  someone who connects now and comes back tomorrow, which is the point.
- **Deleted immediately once a push attempt finishes — success or failure.**
  Waiting out the TTL after the token has already done its one job would be
  exposure with no corresponding benefit.

This is the same reasoning `credentials.rs` already applies to TOTP secrets
and passkeys — a demo session's credential belongs to the session, not to a
person, and its lifetime should be no longer than the thing it exists to let
happen.

### The push is one commit, via the Git Data API

`POST /user/repos` (`auto_init: false`) creates an empty repository; a blob is
created per generated file; one tree is built over all of them; one commit is
created with **no parents** (the repository has no history yet to be a
continuation of); `refs/heads/main` is pointed at it. The alternative — the
Contents API, one `PUT` per file — would create one commit per file, which
misrepresents what a visitor actually did: they made one configuration
choice, and it should read in `git log` as one arrival, not a commit storm.

### Failure modes stay distinct

A repository-name clash, an invalid name, a dead token, a missing scope, a
rate limit, and a network failure are reported as six different errors
(`github_repo_name_taken`, `github_invalid_repo_name`, `github_token_rejected`,
`github_scope_missing`, `github_rate_limited`, `github_network_error`), never
folded into one generic failure. Each has an unrelated fix — rename,
reconnect, wait, or check your own connection — and a flat `500` would hide
which one a visitor is looking at, forcing them to guess.

### The zip download is untouched

`GET /api/starter-kit` remains exactly as it was: unauthenticated, no GitHub
account required. It is the fallback for anyone who does not want to connect a
GitHub account at all, or who is on a network/browser where the OAuth
round-trip is inconvenient. Pushing to GitHub is a second way to get the same
project, not a replacement for the first.

## Consequences

- Two independent sets of GitHub credentials to register and configure per
  deployment, rather than one — a small operational cost for keeping the two
  demos honest about what each actually asks for.
- The GitHub calls are made through a `GitHubApi` trait (`github_api.rs`)
  rather than a bare HTTP client sprinkled through the route handlers, so the
  test suite can substitute a fake and exercise every failure mode above with
  no network access and no HTTP mock server.
- Private-repository support, and a GitHub-App-based token model with
  narrower, revocable, per-repository access, are both left as explicit
  follow-ups rather than folded into this change.
