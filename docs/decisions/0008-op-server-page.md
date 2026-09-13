# 0008 — The OP server page: not in the playground

**Status:** accepted
**Date:** 2026-09-14
**Roadmap:** P6, #50 "Be your own identity provider: an OP server page"

## The question

#50 ends by naming its own prerequisite: decide whether an OpenID Provider demo
belongs in the playground at all, or as a separate example application, before
any of its task list is started. Upstream already ships four runnable
`*_op_server*` examples. This record makes that case, rather than assuming it,
and lands on: **neither.** Point at upstream. Do not build a playground page or
a new example application for this.

## Why the wizard is the wrong home

Verified against the code, not just #50's assertion. `Playground.tsx` is a
fixed three-step sequence — `StepChooseMethods`, `StepSignIn`, `StepDownload` —
and every scenario reaches the visitor through one `Scenario` trait
(`apps/api/src/scenario/mod.rs`) whose only control shapes are `Toggle`,
`SelectOne` and `SelectMany`. A scenario is something a visitor flips on in
step 1 and experiences signing in, in step 2, inside the same session. That
shape is why `ScenarioContext::user_id` is simply the demo session id: every
existing scenario has exactly one party, the visitor, playing both roles a
protocol needs.

An OP has no such single-party reading. #50 states the asymmetry precisely:
"it is only meaningful once something authenticates *against* it." Proving an
authorization-code flow needs a second, independent application acting as
client — a distinct redirect URI, its own PKCE verifier, its own notion of
"logged in" that is not the playground's session. That is not a fourth step
bolted onto the wizard; #50's own task list says as much — "a dedicated route,
outside the three-step wizard" is the first item. Once a scenario needs a
second application, it has stopped being a configuration a visitor tries and
started being a second product the playground would host and operate
alongside the first. Nothing else registered in `ScenarioRegistry` asks for
that, and the fragment model in `0005-starter-kit-model.md` — one linear
builder chain, concatenated fragments — has no representation for "and also
run this second server."

## Why examples, not a live flow, is the right register here

#50's counter-argument is that examples are read while a live flow is used.
That is true and it is not sufficient on its own — it proves the live flow is
more compelling, not that this playground should be the one to host it. The
comparison that matters is cost against what the compellingness buys, and here
the cost is unusually front-loaded for a single page:

- a dedicated route and its own explanatory framing, outside the wizard
- a demo client, maintained as a second application with its own trust
  relationship to the OP, on top of the client the playground already gives
  starter-kit downloads for every other scenario
- discovery and JWKS served and displayed a second time — the API already
  serves `/.well-known/jwks.json` for the resource scenario; an OP would need
  its own discovery document and its own key set, distinct from the
  deployment's signing identity, or the two roles would be confusable on the
  same domain
- mandatory PKCE and RFC 8252 loopback redirect URIs, correctly, on a public
  demo where the "client" is whatever the visitor's browser can stand up
- `OpStore` persistence, meaning a durable per-visitor record where today the
  playground's credential state is scoped to a session and swept on expiry
  (`0002-session-store.md`, `0004`)
- failure-mode surfacing to the same standard as the resource scenario (#52),
  which is its own multi-issue effort
- a starter-kit fragment, which for every other scenario is a few lines
  appended to a linear chain and for an OP is close to a second generated
  project

Every other scenario in P2 answers "does this feature work," in one page,
inside a shared session. An OP answers "can two independent programs agree,"
which is a different kind of demo and, going by #52's and #61's own scope,
roughly the size of another whole scenario's worth of work by itself before a
visitor sees a single authorization code.

## What actually changed since #50 was written, and why it does not flip the answer

#50 was filed before the resource scenario existed in its current form.
Confirmed by reading `apps/api/src/scenario/resource.rs` and
`apps/api/src/signing.rs`: the deployment now has a per-deployment Ed25519
signing key, a real `GET /.well-known/jwks.json`, and a `call` path that
validates through `authkestra-resource`'s `JwksCache` and `IssuerTrustMap` —
`kid` lookup, untrusted-issuer rejection, `require_kid`, all present
(`resource.rs:16-18, 45, 116-196`). #52 argued explicitly that this resource
server is "exactly the counterpart an OP would need to prove itself against,"
and that argument does survive: it is a correct description of what the
resource scenario now offers.

What it does not do is change who the second party is. The resource scenario
validates tokens the *playground itself* minted with its own deployment key —
still one party, the visitor's session, wearing two hats via one process's
`SigningKeys`. An OP proving itself against that resource server would still
need an independent client application in the RFC 8252 sense, issuing its own
authorization requests with its own redirect URI. The resource server being
real now makes a future OP demo's *second half* more credible than it would
have been in P2 — a rotated key would genuinely propagate through
`JwksCache` — but it supplies nothing towards the missing client, which is the
asymmetry that made this large in the first place. #61 (the stale-cache
follow-up) is itself still open work on the resource side alone; treating the
OP page as newly cheap because of #52 undercounts what #52 actually shipped.

## The per-visitor persistence question, and why it argues against building this at all

#50 flags, as an open task, deciding what an OP would store per visitor.
Worth answering here rather than deferring again: an OP that issues real
authorization codes and tokens to a real client must record, at minimum, the
client's redirect URI and PKCE challenge for the lifetime of one authorization
request, and — if the demo is to mean anything past a single round trip — some
notion of which visitor authorized which client, for revocation or a second
look at "what did I just grant." That is a durable, identity-shaped record on
an anonymous, unauthenticated public demo, which is a different risk class
from the session-scoped credentials `credentials.rs` already sweeps on expiry.
Nothing about this is unbuildable — `SqlxOpStore` exists precisely to hold it —
but it is one more reason this is a second product's data model, not a
scenario's.

## Decision

Do not build an OP server demo in this playground, and do not stand up a
separate example application for it either. Point visitors who ask "can
authkestra be an OP" at the upstream `*_op_server*` examples, which already
run and already answer that question at the register examples are good at:
read the code, run it locally, see the discovery document and the tokens.
`#50` is closed by this record rather than carried forward into P6.

### What would change this

- **A second, independently-justified reason to run a real client
  application in this playground** — for instance, if a future scenario
  needed one anyway (a relying-party demo, a DPoP-bound client) — would change
  the cost side of this calculation, because the client would no longer be
  built solely to give the OP something to prove itself against.
- **`OpStore` persistence becoming cheap and clearly scoped** — if a future
  session-store decision (superseding `0002`/`0004`) gives every scenario a
  durable, per-visitor record with a defined retention and deletion story
  anyway, the objection in the persistence section above weakens and should be
  re-weighed on its own.
- **Evidence that visitors are asking and not finding the answer** — usage
  data from P5 showing people arrive at the playground specifically looking
  for "run your own IdP" and bounce, which is the kind of signal P6 is
  explicitly waiting for before anything in that phase is scoped.

Short of one of those, the answer is upstream's examples, and this record is
why.
