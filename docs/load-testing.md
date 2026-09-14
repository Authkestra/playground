# Load and abuse testing

Runbook for `scripts/abuse-profile.mjs`, which exists to close out #35: confirm
that hostile or merely heavy traffic gets throttled rather than turning into an
outage or a surprise invoice. Read this before running it against anything
that isn't your own laptop.

The script's own header comment (`scripts/abuse-profile.mjs`) has the detail on
which endpoints it hits and why; this document is the "how do I run it and
what do I do with the answer" half.

## Quick start

```sh
# A realistic traffic sample — several visitors clicking through the site.
node scripts/abuse-profile.mjs --target https://play.authkestra.com --profile interactive

# Scripted hammering of the endpoints the tighter rate limit protects.
node scripts/abuse-profile.mjs --target http://localhost:8080 --profile abuse
```

`--target` is required; there is no default, on purpose. There is no way to
run this without saying, explicitly, what it points at.

`--profile abuse` refuses to run against `play.authkestra.com` or any
`*.onrender.com` host unless you also pass `--yes-really`. That is not a
formality — a concentrated abuse run against the real deployment throttles
real visitors and, if it lands on a provider-facing endpoint, spends someone
else's quota. If you genuinely mean to abuse-test the live deployment (for
example, deliberately, as part of the kill-switch procedure below, during a
maintenance window), read the refusal message before overriding it.

Add `--report` to either profile to print the same output without the exit
code mattering — useful for a one-off look at current numbers without wiring
it into anything that checks the result.

## Reading the output

```
profile: abuse
target: http://localhost:8080/
requests sent: 400
wall time: 0.34s (achieved ~1162.8 req/s)
status codes:
  200: 5
  429: 395
latency: p50 13ms  p95 23ms  p99 33ms
429s: 395   5xx: 0

guardrail held.
```

- **status codes** is the full distribution, not just the interesting ones —
  a stray `404` here usually means a route moved and the script's endpoint
  list needs updating, not that anything is abusive.
- **429s** and **5xx** are pulled out separately because they are the two
  numbers the exit code is actually about (see below). `0` as a status code
  means the request never got a response at all — a connection refused or
  timed out, which almost always means `--target` is wrong or the service is
  down, not that a guardrail did anything.
- **latency** is wall-clock per request, including the ones that got a `429`
  back quickly — a fast `429` is the limiter working, not a fast success.

### Exit code

Non-zero means a guardrail failed, and it is one of exactly two things:

1. **Any `5xx`.** The service is supposed to throttle under load, not fall
   over. A `500` under an abuse run is a bug in the handler, not a rate-limit
   problem — the limiter's job ended the moment it let the request through.
2. **An abuse run with zero `429`s.** The whole point of `--profile abuse` is
   to send enough traffic, concentrated enough, that the sensitive bucket's
   burst of 10 gets exceeded. If it never does, the limiter isn't holding —
   which is a worse finding than seeing a wall of `429`s, not a better one.

The interactive profile is held to the first rule but not the second: it is
supposed to look like a normal visit, and a normal visit hitting a `429` is
itself a finding (the burst is under-sized), just not one this script decides
on your behalf — read the status distribution and use your judgement.

## The two profiles

**Interactive** simulates concurrent visitors, each making one realistic pass
through the site — list scenarios, get a session, toggle a control, read the
diff, try one ceremony step, and (one visitor in five) download the starter
kit. `apps/api/src/lib.rs` sizes both rate-limit buckets specifically so this
never gets throttled ("a visitor clicking through every scenario is never
throttled"); if a healthy target ever 429s this profile, treat it as a bug in
the burst sizing, not in the traffic.

**Abuse** is scripted, concentrated hammering of the sensitive bucket — the
one guarding `POST /api/scenarios/:id/action/:action` and
`GET /api/starter-kit`, which create credentials or do the service's most
expensive work. It sends well past the documented burst of 10 as fast as it
can, and expects `429`s back.

Both profiles deliberately never drive `POST /api/scenarios/oauth/action/*`,
`POST /api/scenarios/captcha/action/*`, the `/auth/login` and `/auth/callback`
navigation routes, or anything under `/api/github/`. Those reach a real third
party — GitHub, an OAuth provider, or a captcha verifier — and a script
sending them repeatedly spends *that provider's* rate limit, not just ours.
Enough of that from an automated run is how a provider suspends the
deployment's app. The script hammers the endpoints that share the same
sensitive bucket without that risk instead: TOTP provisioning, a WebAuthn
challenge, JWT issuance, and the starter-kit download.

## What this cannot verify

Three of #35's acceptance points need a human with access this script
doesn't have. Automating around that would just be a false green tick, so
they stay manual. Each has a concrete procedure below — do them, don't assume
them.

### 1. Billing alerts actually fire

A load test proves the *rate limiter* holds; it says nothing about whether a
billing alert would have caught it if the limiter hadn't. Test the alert path
itself, on whichever host is live:

- **Render** — Dashboard → the service → **Metrics**, and account-level
  **Billing** → usage alerts. Set a deliberately low threshold temporarily,
  generate enough traffic to cross it (an extended `--profile abuse` run
  against a non-production target works for volume, but the alert is
  account-scoped, so this has to be run against whichever deployment the
  alert actually watches), and confirm the notification arrives before
  restoring the real threshold.
- **Cloud Run / GCP** — Billing → **Budgets & alerts**. Create or edit a
  budget with a threshold low enough to trigger from current spend, confirm
  the email/Pub/Sub notification actually lands, then restore it. GCP budgets
  check spend periodically rather than in real time, so "it fired" needs to
  be confirmed after a delay, not assumed the moment the threshold is
  crossed.

Either way, the deliverable is a notification you watched arrive, not a
configured threshold you're trusting.

### 2. The kill switch actually degrades traffic under load

`killswitch.rs`'s in-process cache has a 5 second TTL, so a flip is visible
everywhere within 5 seconds — that is documented behaviour, not tested
behaviour, until it's watched happen under real concurrent load. Procedure:

```sh
# 1. Start an abuse run against the target deployment. Use --yes-really only
#    if the target is play.authkestra.com/onrender.com and you mean it — this
#    is exactly the deliberate, supervised abuse run that flag exists for.
node scripts/abuse-profile.mjs --target <api-url> --profile abuse --duration 30 &

# 2. A few seconds in, flip the switch mid-run:
curl -s -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"demo_enabled": false}' \
  <api-url>/admin/kill-switch

# 3. Let the run finish, then flip it back:
curl -s -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"demo_enabled": true}' \
  <api-url>/admin/kill-switch
```

What "worked" looks like: within 5 seconds of the `curl` in step 2, requests
that were getting `200`/`429` start getting `503` (`demo_disabled` — see
`apps/api/src/error.rs`) instead, and the run's final status distribution
shows a visible block of `503`s starting partway through rather than at the
beginning or not at all. A `503` appearing here is the switch working, not a
guardrail failure — **don't** run this against the automated pass/fail exit
code; a deliberately-triggered `503` would trip the "any 5xx is a failure"
rule for the wrong reason. Read the printed status distribution by eye for
this one, or run with `--report` so the exit code is ignored entirely.

Confirm the frontend degrades to explainer-only mode too (controls render as
unavailable, per `apps/api/src/routes.rs`'s `specs_for` — that half is outside
what an API-level load test can see at all).

### 3. Third-party quota consumption

Both profiles refuse to touch the OAuth and captcha endpoints precisely so
this script cannot answer this one — checking would mean spending the
quota it's trying to protect. Check the providers' own dashboards instead
after any real traffic spike (organic or a deliberately-authorised abuse run
against the live deployment):

- **GitHub** (sign-in app and the separate push app, `GITHUB_KIT_CLIENT_ID`)
  — each app's rate-limit headers are visible via the GitHub API, or
  `gh api rate_limit` using a token that authenticates as that app's
  installation.
- **Google, Discord** (OAuth) — their respective developer consoles show
  request volume per app.
- **Turnstile / hCaptcha / reCAPTCHA** — each vendor's dashboard shows
  `siteverify` call volume for the configured site key.

If any of these is near a limit, the fix is upstream of this script — reduce
`SENSITIVE_BURST`/`SENSITIVE_REPLENISH_SECS` in `apps/api/src/lib.rs`, not
loosen this document's rule about which endpoints get driven.

## Troubleshooting

- **Refused immediately, before sending anything** — `--profile abuse`
  against a production-looking host without `--yes-really`. Working as
  intended; see Quick start.
- **`--target is not a valid URL`** — include the scheme (`https://...`), not
  just a host name.
- **A wall of status `0`** — nothing answered. Check the host is reachable
  and the port is right before assuming anything about rate limits.
- **Interactive profile shows `429`s** — either the standard/sensitive burst
  in `apps/api/src/lib.rs` has shrunk since the numbers this script's header
  comment cites, or something ahead of the API (a proxy, Cloudflare) is
  bucketing differently than expected — check with
  `GET /admin/client-ip` (documented in `docs/deployment.md`) before assuming
  the script itself is at fault.
