# Launch plan — issue #37

Issue #37 has five tasks. Two of them ("docs page on authkestra.com" and "launch
post") are content someone has to write in their own voice; this document does
not attempt either. What follows is the outline for the launch post (§0 —
claims and evidence, not prose) plus the three operational tasks: pre-flight,
anticipated scrutiny, and triage. All of it is written against what is actually
in this repo today, 2026-09-14.

Two facts shape everything below and are worth stating up front rather than
burying:

- **#34 (security review) and #35 (abuse/load test with cost guardrails) are
  in flight, not finished.** `docs/security-review-2026-09.md` and
  `docs/load-testing.md` arrive with them; read both before working through §1,
  and treat a reference here to either as pointing at the merged version rather
  than a promise. Issue #37's own fourth task — "confirm the kill switch, rate
  limits and alerts are live" — still cannot be ticked off by citing a review.
  A review says the code is sound; only the manual checks in §1 say the live
  deployment is configured the way the code assumes. Those are different
  claims, and launch depends on the second one.
- **`play.authkestra.com` is not live.** #5 (Vercel + Cloudflare DNS) is open
  and `blocked:external`. The README says so, and the launch post's links have
  to point at whatever *is* live today (`playground-web-opal.vercel.app` and
  the Render API), not at the vanity domain, unless #5 lands first.

---

## 0. Launch post outline (not the post itself)

This is the shape of the argument and the evidence for each beat, in the order
that earns the next one. Write it in your own voice — this is scaffolding, not
copy. Anywhere marked `[maintainer: ...]` needs a real number, opinion, or
anecdote this document has no business inventing.

1. **What it is, stated exactly.** One sentence, and the repo already has it —
   the README's own framing: "an interactive playground and starter-kit
   generator for authkestra, a framework-agnostic authentication orchestrator
   for Rust." Configure auth, see the diff, test it live, download a working
   Rust project. Don't editorialise this beat; it's already precise.

2. **The Rust-adoption angle** — issue #37 names this explicitly and it's the
   real hook for r/rust specifically. The honest version of the claim is
   narrower than "we made auth easy": it's that the playground's *output* is
   not a marketing artefact but the same dependency graph and builder chain
   the playground itself runs, which is what `0005-starter-kit-model.md`'s
   parity test (#33) and the CI compile matrix (#32) exist to guarantee.
   `[maintainer: state the adoption problem you're actually solving for —
   trust in generated code, unfamiliarity with the framework's builder API,
   something else]`.

3. **Show, don't tell: the diff.** The mechanism worth describing concretely —
   toggle a scenario, the diff viewer renders the real `Cargo.toml`/builder
   changes that configuration implies (`ScenarioSpec` + `Scenario::consequences()`
   per `0005-starter-kit-model.md`), then download and it's the same text,
   compiled and tested in CI. This is the load-bearing claim of the whole
   post and the one a Rust reader will try hardest to break — see §2 for what
   that scrutiny actually finds.

4. **What's real today vs. what's built-but-waiting.** State the README's
   Status table plainly rather than letting a commenter discover it:
   TOTP, passkeys, and the resource server work end to end; OAuth and bot
   protection are built, tested, and waiting on credentials only the
   maintainer can register. This is honesty as a feature, not a caveat to
   bury in paragraph nine — see §2 for why leading with it is the right
   posture.

5. **The guardrails, briefly.** One or two lines: rate-limited, kill-switched,
   stateless (a redeploy costs nothing a visitor did). Detail belongs in the
   repo's own docs (README "Safety" section, `docs/deployment.md`), not the
   post — link rather than restate.

6. **The starter kit as the actual deliverable.** The playground is a demo;
   the zip is the thing someone keeps. Worth naming that the generated
   project is flat, single-crate, and `cargo run` is the only instruction
   (`0005-starter-kit-model.md`'s file tree) — a design choice, not an
   omission.

7. **Call to action.** Link to the live playground (whichever origin is
   actually serving traffic that day — see the pre-flight link check in §1),
   the starter-kit download, and `marcjazz/authkestra` itself. `[maintainer:
   decide whether to ask for a GitHub star here — #46 already built an
   on-download ask, so the post doesn't have to duplicate it]`.

8. **Where to file what.** One line, because it will get asked in comments
   regardless: bugs in the generated code or the framework's own behaviour go
   to `marcjazz/authkestra`; bugs in the playground itself (this repo) go
   here. See §3 for the fuller version of this distinction.

Do not include: adoption numbers, star counts, or user quotes — none exist
in this repo to cite, and inventing placeholders reads worse than omitting
the section.

---

## 1. Pre-flight checklist

Each item names what to check and how, against what actually exists in this
repo — not generic launch advice.

### Kill switch

- [ ] **`ADMIN_TOKEN` is actually set on the production Render service.**
  `apps/api/src/lib.rs` (`build_router`) only mounts `admin_router()` — the
  routes below — when `settings.admin_token.is_some()`; otherwise it logs
  `"ADMIN_TOKEN unset; admin kill-switch endpoint not mounted"` and there is
  no way to flip anything at runtime. Check the Render dashboard's env vars,
  or watch the boot log for that warning — its absence is the confirmation.
- [ ] **The switch actually flips within 5 seconds.** Call it for real:
  ```sh
  curl -s -X POST -H "Authorization: Bearer $ADMIN_TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"demo_enabled": false}' https://<api>/admin/kill-switch
  # then, within a few seconds:
  curl -s https://<api>/health   # demo_enabled should read false
  ```
  Flip it back (`{"demo_enabled": true}`) immediately after — this is a live
  production toggle, not a dry run. `apps/api/src/killswitch.rs`'s own doc
  comment gives the reasoning for the 5-second cache TTL; confirm the number
  matches reality rather than trusting the comment.
- [ ] **A per-scenario disable works independently of the global one.**
  `{"scenarios": {"totp": false}}` against the same endpoint, then check
  `GET /api/scenarios` shows TOTP unavailable while everything else still
  works. `killswitch.rs`'s tests already prove this in isolation; this step
  proves it against the deployed store (Redis), not just in-process.
- [ ] **Degraded mode reads as intentional, not broken.** With the switch off,
  load the actual frontend and confirm controls render as unavailable with an
  explanation, per the design stated in `killswitch.rs`'s module doc. This is
  a visual check, not just an API check — it's the thing that keeps a
  necessary mid-launch kill from reading as an outage to whoever's watching.
- [ ] **Redis unreachable does not fail open.** Not something to test against
  production, but worth confirming you understand the failure mode before you
  need it under pressure: `snapshot()` falls back to the last cached value,
  never re-reads the environment seed once the store has been written to, and
  only returns the (enabled-by-default) env seed on a cold cache with an
  unreachable store — a state the module's own comment says is "close to
  unreachable in practice" because `open_state_store` connects eagerly at
  boot. Know this before 2am, don't verify it at 2am.

### Rate limits

- [ ] **The two buckets are what you think they are.** `apps/api/src/lib.rs`
  (around line 236): standard bucket refills one token every 2s with a burst
  of 30; sensitive bucket (OAuth login, GitHub push, anything hitting a
  third party) refills one every 5s with a burst of 10. Confirm these
  constants haven't drifted from what this document assumes by grepping
  `STANDARD_REPLENISH_SECS|STANDARD_BURST|SENSITIVE_REPLENISH_SECS|SENSITIVE_BURST`
  before launch, since a future change to these numbers won't update this file.
- [ ] **`TRUSTED_CLIENT_IP_HEADER` is actually set in production, not just
  documented as something you should set.** This is the single item most
  likely to be silently wrong under real launch traffic, and it degrades
  quietly rather than loudly: unset, the limiter falls back to the rightmost
  `X-Forwarded-For` entry, which `docs/deployment.md` calls "the safe
  default" but also means every visitor behind the same proxy hop can share
  a bucket — coarse, not a bypass, but exactly the kind of thing that turns
  "HN sent 200 visitors" into "the first 30 get through and the rest see
  429s that have nothing to do with abuse." Check it for real, against the
  live deployment, not from memory of what should be right:
  ```sh
  curl -s -H "Authorization: Bearer $ADMIN_TOKEN" https://<api>/admin/client-ip | jq
  ```
  This reports every candidate header that actually arrived and which one
  the limiter is using. `docs/deployment.md` notes Render fronts traffic with
  Cloudflare, so `cf-connecting-ip` is the likely right answer there — but
  the whole point of this endpoint is that vendor documentation and reality
  don't reliably agree, so read the actual answer rather than assuming
  Cloudflare. **Do this after the last infra change before launch, not once
  during initial setup** — a proxy change since then would silently
  invalidate it.
- [ ] **A 429 renders as the friendly message, not a raw error.** Trigger one
  (script a burst past the standard bucket's 30) and confirm the frontend
  shows the JSON body's `detail` string rather than a generic failure — the
  shape is documented in `docs/api-contract.md` specifically so the UI
  doesn't have to parse prose.

### Alerts and cost guardrails

- [ ] **#35 (abuse/load test) has not been run as of this writing.** Its own
  acceptance criterion — "a hostile traffic profile produces throttling and
  alerts, not a surprise invoice or an outage" — is exactly what issue #37's
  fourth task asks you to confirm before posting. If #35 closes before
  launch, defer to whatever it found instead of this checklist. If it does
  not close before launch, see §3's "abuse before load test" plan — do not
  post while pretending #35 is done.
- [ ] **Billing alerts fire, tested rather than assumed.** #35 says this
  explicitly ("verify billing alerts actually fire (test them, don't
  assume)") for the same reason a smoke-tested deploy exists: a green
  dashboard is not proof of a working alert, the same way a green deploy is
  not proof of a working service (`docs/deployment.md`'s closing line, about
  the health-check smoke test). Whatever billing alerting exists on
  Render/Vercel/the OAuth providers/captcha providers, trigger it once
  deliberately before relying on it.
- [ ] **Third-party quota is sane under the sensitive bucket's ceiling.**
  OAuth login and the GitHub push both live in the sensitive bucket
  (5s/10 burst per IP) specifically because they burn provider quota — see
  the comment at `apps/api/src/lib.rs` around the sensitive `GovernorConfig`.
  That bounds one IP's damage; it says nothing about aggregate traffic across
  many IPs against GitHub/Google/Discord's own app-level rate limits, which
  is exactly what #35's "sanity-check third-party quota consumption... under
  the abuse profile" task is for. Until that's run, know what each OAuth
  app's own dashboard shows for remaining quota so a spike is recognisable
  rather than mysterious.

### Links and domains

- [ ] **The launch post's links point at what's actually live.** #5
  (`play.authkestra.com` DNS) is open and `blocked:external` as of this
  writing — the README says the domain "is not wired up yet." Link to
  `playground-web-opal.vercel.app` and the Render API unless #5 has landed by
  the time the post goes out. If it has landed, re-verify with the "moving to
  play.authkestra.com" checklist in `docs/deployment.md` (`ALLOWED_ORIGINS`,
  `NEXT_PUBLIC_API_BASE_URL`, `WEBAUTHN_ORIGIN`/`WEBAUTHN_RP_ID`,
  `COOKIE_SAMESITE`) — three of those four fail *silently*, per that doc's
  own table, so a same-day domain move without re-checking them is the kind
  of thing that reads as "the site is broken" within an hour of posting.
- [ ] **`GET /health` and a credentialed CORS check pass against whatever
  origin the post actually links to.** `docs/deployment.md`'s "Verifying a
  deployment" section gives both commands — run them against the real URL in
  the post, not the URL from the last time this was checked.

### Load testing

- [ ] Run `node scripts/abuse-profile.mjs` against the live service and read
  `docs/load-testing.md` first — both arrive with #35. A clean abuse run
  supersedes the rate-limit spot checks above; a run that produces no `429`s
  is the limit failing, not the limit being generous. Cross-check what it
  reports against the bucket constants named here before posting.

---

## 2. Scrutiny to expect, and the honest answers

#37 names HN and r/rust specifically and expects scrutiny of the generated
code. A sceptical Rust reader's instinct is to find the one thing that's
oversold and use it to discount everything else — so the right posture is to
name every gap below before a comment does, in the launch post itself where
it's relevant (see §0 beat 4).

**"Does the facade crate actually give you this, or are you hiding extra
dependencies?"** — Fair question, and the honest answer is in
`docs/decisions/0001-dependency-and-tls-baseline.md`: the `authkestra` facade
does **not** expose `webauthn`, `totp`, `captcha`, `op`, `devsig`, or any store
backend — those live on `authkestra-engine`, `authkestra-axum`, and
`authkestra-store-sqlx` directly, and the facade only pulls them in as
`[dev-dependencies]` for its own examples. This playground depends on the
sub-crates directly rather than the facade, for exactly this reason, and the
starter-kit generator emits that same block verbatim. If someone asks "why
aren't you just using `authkestra`", the answer is that the facade alone
would not compile what's being demonstrated.

**"TLS backend — did you just reach for the easy default and call it a
choice?"** — Yes, deliberately, and the decision record says so: `0001`
rejected `rustls-no-provider` for v0 because it requires installing a
`rustls::CryptoProvider` before any HTTP client is constructed or `reqwest`
panics at construction — "a sharp edge to hand a newcomer in a generated
starter kit." `rustls-aws-lc-rs` needs a C toolchain, which the container
already has for other reasons. This is a real trade-off, stated as one, with
a documented escape hatch (turn defaults off, install a provider yourself) for
musl targets or a `cargo-deny` policy that bans `aws-lc-rs`. Don't claim this
is the only correct answer; claim it's the right one for a starter kit whose
whole point is that it runs.

**"Is the diff real, or is it a mockup that doesn't match what gets
generated?"** — This is the claim most worth being able to defend precisely,
because it's the one the whole post leans on (§0 beat 3). The honest, specific
answer: `Scenario::consequences()` is the single source of the crates,
features, and routes both the diff *and* the generator read from
(`0005-starter-kit-model.md`), and a test asserts the generated `Cargo.toml`'s
feature list matches the diff's `crates` entry for the same configuration —
"one source of truth," in that document's words, specifically to prevent the
diff promising one dependency set and the download shipping another. If asked
for the test, point at it rather than asserting from memory — `[unverified:
this document has not located the exact test file/name for #33's parity
assertion; find and cite it directly rather than repeating this paraphrase]`.

**"OAuth and bot protection are green in your table — are they actually
working, or is this vapourware with a checkmark?"** — Neither: they're
"built, tested, and waiting on credentials," which is a specific and checkable
claim, not marketing language. The README states this plainly and #37's own
posture should match: the scenario, its actions, its diff, and its
starter-kit fragment are shipped and tested; what's missing is the maintainer
registering provider credentials, which absent-credential handling reports as
"not configured" rather than erroring. Lead with this rather than waiting for
someone to notice the greyed-out toggle.

**"reCAPTCHA specifically — I tried it and it silently failed / the widget
solved and then errored."** — This is the sharpest edge in the repo and worth
naming unprompted rather than waiting for the bug report, per #51: the
deployment's reCAPTCHA keys are Enterprise, the engine speaks only the legacy
`siteverify` protocol, and presence-of-both-halves is all the credential guard
can check — so the control offers reCAPTCHA, the widget renders, the visitor
solves it, and verification fails at the last step with nothing explaining
why. Per #51's own "do this now," the mitigation is to unset
`RECAPTCHA_SECRET_KEY` in production so the control simply stops offering
reCAPTCHA (Turnstile and hCaptcha still verify the classic way) — **confirm
this has actually been done before launch**, since #51 describes it as the
fix but this document has not verified the environment variable is unset
today. If it hasn't been unset, this is a live, reproducible bug a launch
audience will find within the first hour, not a hypothetical.

**"You're a wrapper — what does this framework actually save me, versus
writing the axum/webauthn-rs/oauth2 plumbing myself?"** — This is a real
question this document cannot answer on the maintainer's behalf; it's a
product-value claim, not a fact in the repo. `[maintainer: this is the
question worth having a crisp two-sentence answer ready for, since it's the
one a Rust audience asks first — not because the repo lacks an answer, but
because the right answer is a judgment call about the framework's value, not
something derivable from file paths]`.

**"MSRV / unsafe / dependency audit — did you actually run `cargo deny`, or
is that aspirational?"** — CI runs `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test`, `cargo llvm-cov`, and `cargo deny check`, per the
README's CI section — this is checkable in the Actions tab, so link it rather
than asserting it. The one honest caveat: the coverage gate is 50%, stated in
the same section as explicitly lower than the framework's own 84%, with "and
should ratchet up" attached. Don't round 50% up to sounding like 84%.

---

## 3. Triage plan for the first wave

Existing labels, not a new scheme (`gh label list`): `area:api`, `area:web`,
`area:infra`, `area:starter-kit`, `area:docs`, `area:scenario`, `type:feature`,
`type:chore`, `type:test`, `type:security`, `type:research`, `blocked:external`,
`bug`, `question`, `good first issue` / `good-first-issue` (both exist —
prefer `good-first-issue`, the one actually used on recent issues per
`roadmap.json`), `help wanted`, `duplicate`, `invalid`, `wontfix`.

### Same-day response vs. not

**Same-day, always:**
- Anything that looks like `type:security` — a public auth-framework demo
  is, in this repo's own words (issue #34), "a uniquely embarrassing place to
  get owned." Triage first, fix or mitigate fast, disclose per whatever norm
  the maintainer wants (not specified anywhere in this repo — decide before
  launch, not during it).
- A report that the kill switch, rate limiting, or the admin endpoints are
  not behaving as documented — these are the load-bearing safety mechanisms
  the whole launch depends on (§1).
- A reCAPTCHA failure report, if `RECAPTCHA_SECRET_KEY` still holds an
  Enterprise key at launch time (see §2) — this is a known, named, expected
  failure mode, so triage is instant: confirm it matches #51's description,
  apply #51's fix (unset the key), close or point at #51.

**Not same-day, and saying so is fine:** `good-first-issue`-shaped requests,
`area:starter-kit` feature asks for combinations outside v0's scope (Actix,
non-SQLite databases — both explicitly out of scope per
`0005-starter-kit-model.md`), and anything `type:research`-shaped. A same-day
promise on everything is how a first wave burns out the one person answering
it.

### Framework bug vs. playground bug

This will be the single most common confusion, because it's the README's
opening point and a first-time visitor has no reason to have absorbed it:
"This repo is not the framework. authkestra itself... lives at
`marcjazz/authkestra`... This repo is only the playground that demonstrates
it and the generator that emits starter projects."

The practical test: **does the bug live in what the playground *built* (the
Axum routes, the diff engine, the session/kill-switch/rate-limit machinery,
the Next.js UI, the starter-kit generator's own concatenation logic), or in
what authkestra *does* when the playground calls it (a builder method
behaving wrong, a captcha verifier's protocol, WebAuthn ceremony logic,
OAuth2 flow internals)?** If it's the latter — the generated code is faithful
to what the framework does, and the framework does the wrong thing — it's
`marcjazz/authkestra`'s issue, not this repo's. Redirect there, link the
specific behaviour, and label it here as `invalid` or close it with a comment
rather than leaving it open to be re-triaged later. If it's ambiguous, default
to reproducing locally against the pinned `0.8.0`/`0.8.1` dependency before
deciding — guessing wrong in either direction wastes someone's time on the
other side of the redirect.

### A report that needs credentials nobody but the maintainer has

OAuth, GitHub push, and captcha all gate on credential pairs only the
maintainer holds (README's environment table; `docs/deployment.md`'s
credential section). If a report needs reproducing against live
`GITHUB_CLIENT_ID`/`GOOGLE_CLIENT_ID`/`DISCORD_CLIENT_ID`,
`GITHUB_KIT_CLIENT_ID`, or any `_SITE_KEY`/`_SECRET_KEY` pair to confirm —
label it and hold it for the maintainer rather than asking a triager to
speculate about behaviour they cannot reproduce. Two categories are common
enough to name directly:
- **"OAuth/bot-protection doesn't work"** where the honest first check is
  whether it's simply not configured yet (reports itself as unavailable, not
  broken) versus configured-and-failing (the reCAPTCHA case above is the one
  known instance of the latter).
- **A provider-side registration problem** (wrong callback URI, wrong scope,
  a console-side key type mismatch) — these need the provider's own
  dashboard, which only the maintainer can open. `blocked:external` fits if
  the fix genuinely depends on the provider's own console/support, not just
  on the maintainer's time.

### If the abuse profile arrives before the load test was run

This is the live state of the repo as of this writing — #35 is open, neither
a load test nor its writeup exists. If real hostile-shaped traffic shows up
before #35 has been run deliberately:

1. **Treat it as the load test, unplanned.** Watch the same things #35 would
   have measured: do the two rate-limit buckets actually hold (§1), does the
   kill switch still flip within its 5-second cache TTL under load, does
   third-party quota (OAuth apps, captcha `siteverify`) stay within bounds.
2. **If throttling is holding and nothing is falling over,** let it run and
   capture what it shows — this is free data for #35 later, and premature
   intervention loses it.
3. **If it is not holding** — the sensitive bucket's tighter ceiling
   (5s replenish / 10 burst, `apps/api/src/lib.rs`) exists precisely because
   OAuth and GitHub-push burn third-party quota, so a failure here shows up
   as provider-side errors or exhausted app quota before it shows up as this
   service falling over. That is the trigger for §4 — flip the kill switch
   rather than waiting for #35 to tell you it was necessary.
4. **Write down what happened** in #35 itself (or a new issue linked from it)
   afterwards — an unplanned abuse event with kill-switch and rate-limit data
   attached is close to what #35 asked for, and it should close #35 rather
   than be discarded once the emergency is over.

---

## 4. Rollback: what "switch it off" means

There is no "take the site down" lever in this codebase, by design, and that
absence is worth stating plainly rather than treating as a gap. The kill
switch (`apps/api/src/killswitch.rs`) does not remove the service or return
errors to every visitor — it flips `demo_enabled` (global) or a specific
scenario id (`disabled_scenarios`) to off, and the frontend's job in that
state is to render those controls as **unavailable with an explanation**,
which the module's own doc comment states as the goal: "it should read as
intentional, not broken."

**To flip it:**
```sh
curl -s -X POST -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"demo_enabled": false}' https://<api>/admin/kill-switch
```
or, to disable one scenario without taking down the rest —
`-d '{"scenarios": {"<id>": false}}'` (ids from `GET /api/scenarios`; the live
ones are `totp`, `passkeys`, the resource-server scenario, the OAuth
providers, and bot protection — see the README's Status table for which are
actually reachable today).

**What this buys, concretely:**
- Takes effect everywhere within 5 seconds (the in-process cache TTL) —
  slower than instant, faster than a redeploy, and durable: the state lives
  in Redis, survives a restart, and is not undone by the next deploy the way
  an environment-variable change alone would be (`DEMO_ENABLED` only seeds
  the *first* boot; after that the stored state is authoritative).
- Costs nothing a visitor did — sessions, credentials, and config all still
  live in Redis untouched (`0002-session-store.md`), so re-enabling puts
  everyone back exactly where they were.
- Fails safe if Redis itself is having a bad day: per §1's Redis note, the
  switch falls back to its last known cached value rather than reopening
  flows just because a dependency blinked.

**What it does not do:** stop the frontend from being reachable, stop
`/health` from answering, or stop static/explainer content from loading —
those are deliberately unaffected, because the whole point is a demo that
degrades rather than a site that disappears. If the situation genuinely
requires taking the service off the internet entirely (a compromise, not an
abuse spike), that is a Render/Vercel-level action outside this codebase, not
a kill-switch call — decide before launch whether that authority sits with
the same person who holds `ADMIN_TOKEN`.

**Rollback of a bad deploy** is separate from the kill switch and already
described in the README's Deployment section: Render only redeploys when CI
passes (`autoDeployTrigger: checksPass`), so a broken build cannot reach
production on its own — but a build that passes CI and is still wrong in
production is a `git revert` and a normal merge to `main`, not an admin-token
action.
