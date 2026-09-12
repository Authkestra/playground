# Deploying the playground

The API is a stateless container. All state lives in Redis with a TTL
(`docs/decisions/0004-stateless-service-on-redis.md`), so the host may scale to
zero, recycle instances, or run several — none of it loses a visitor's work.

That means the only hard requirement is **a Redis instance**. The compute is
interchangeable.

## 1. Redis

Any Redis reachable over TLS works. Free tiers that need no card:

- **Upstash** — create a database, copy the `rediss://` connection string.
- **Redis Cloud** free tier — same idea.

Set `REDIS_URL` to that string. `rediss://` (two s's) selects TLS; the service
logs which transport it negotiated at boot.

> `REDIS_URL` unset falls back to an in-process store so `cargo run` needs no
> infrastructure. It is logged as a warning and **must not** be used with more
> than one instance — visitors would get inconsistent state with no other
> symptom.

The service connects and `PING`s at boot, so a bad URL fails immediately rather
than on a visitor's first request.

## 2. Compute

### Render (no card required)

`render.yaml` is a blueprint: in the dashboard choose **New + → Blueprint**,
point it at this repo, and it builds `apps/api/Dockerfile` and redeploys on
every push to `main`. No CI secrets are stored here.

Set the `sync: false` variables in the dashboard (see the file for the list).

**Cost of the free plan:** the service spins down after inactivity and takes
roughly 50 seconds to come back. Visitors keep their configuration across it —
that is what statelessness bought — but the first click after a quiet period is
slow. For something people are invited to try, that is the main argument for the
alternative below.

### Cloud Run (better cold start, needs billing enabled)

`.github/workflows/deploy-cloudrun.yml` deploys with Workload Identity
Federation rather than a stored service-account key. It is `workflow_dispatch`
only until its three secrets exist, so it cannot fire half-configured.

Cold start for this binary is about a second rather than ~50. GCP's free tier is
genuinely free at demo scale, but it requires a billing account on file.

### Fly.io

`fly.toml` still works and the container is unchanged; Fly now requires a card.
The single-instance and no-autostop notes in that file are historical — they
were load-bearing only while state lived in process memory.

## 3. Frontend

Vercel, root directory `apps/web`, deployed by
[`.github/workflows/deploy-web.yml`](../.github/workflows/deploy-web.yml) rather
than by Vercel's Git integration — turn that integration off, or both deploy on
every push and race. The workflow needs `VERCEL_TOKEN`, `VERCEL_ORG_ID` and
`VERCEL_PROJECT_ID` as Actions secrets; the last two come from
`.vercel/project.json` after `vercel link`. Without them it warns and skips
rather than failing, so forks stay green.

Two settings have to agree with the API:

| Where | Variable | Value |
| --- | --- | --- |
| Vercel | `NEXT_PUBLIC_API_BASE_URL` | the API's origin |
| API | `ALLOWED_ORIGINS` | the frontend's origin(s), comma-separated |

**A mismatch does not raise an error.** CORS here is credentialed, because the
demo session rides in an HttpOnly cookie, so the browser simply blocks the
request and the site reads as "API unavailable". Origins are compared exactly:
no trailing slash, scheme included.

### Cross-site cookies

If the frontend and API sit on **different registrable domains** — say
`*.vercel.app` and `*.onrender.com` — the session cookie must be
`SameSite=None`, which is the default whenever `COOKIE_SECURE=true`.

`Lax` cookies are only sent on same-site requests and top-level navigations, so
with `Lax` across sites every API call arrives without a session, gets a fresh
one, and the visitor's configuration silently never persists. The visible
symptom is a control that appears to do nothing: flip a toggle and "Try it"
still reports it is switched off.

Once both sides share a domain (`play.authkestra.com` and
`api.play.authkestra.com` are same-site), set `COOKIE_SAMESITE=lax` — it is the
stricter choice and removes the cross-site exposure entirely.

### Checklist for moving to play.authkestra.com

Four settings change together, and three of them fail *silently* if missed:

| Setting | New value | If you forget |
| --- | --- | --- |
| `ALLOWED_ORIGINS` (API) | `https://play.authkestra.com` | Browser blocks every request; site reads as "API unavailable" |
| `NEXT_PUBLIC_API_BASE_URL` (Vercel) | the API's new origin | Frontend calls the old host |
| `WEBAUTHN_ORIGIN` / `WEBAUTHN_RP_ID` (API) | `https://play.authkestra.com` / **`authkestra.com`** | Ceremonies fail with a vague browser error — **and every existing passkey stops working, because a passkey is bound to the RP ID that created it** |
| `COOKIE_SAMESITE` (API) | `lax` | Nothing breaks; you just keep the looser cross-site cookie |

`OAUTH_REDIRECT_BASE` only changes if the *API* host moves; the callback URIs
registered with each provider must match it exactly.

## 4. WebAuthn

| Variable | Value |
| --- | --- |
| `WEBAUTHN_ORIGIN` | the **frontend's** origin, exactly as the browser sends it |
| `WEBAUTHN_RP_ID` | the frontend's domain (derived from the origin if unset) |

The ceremony runs in the browser at the page's origin, so the relying party is
the *frontend*, not the API. Pointing it at the API host is the usual cause of a
ceremony failing with a deliberately vague browser-side error.

A passkey is bound to the RP ID that created it, so **changing domains means
every visitor re-registers.**

The RP ID may be a **registrable suffix** of the origin, so `authkestra.com`
works for a page served at `play.authkestra.com`. Prefer the suffix: passkeys
then survive a later move to another `*.authkestra.com` subdomain, where pinning
`play.authkestra.com` would invalidate all of them.

`WEBAUTHN_EXTRA_ORIGINS` accepts additional origins — useful for a local
frontend against a deployed API. It cannot bridge different registrable domains,
because the browser separately requires the RP ID to be a suffix of the page's
own origin.

## 5. Everything else

| Variable | Why |
| --- | --- |
| `PUBLIC_BASE_URL` | This API's own base URL, no trailing slash. The `iss` of every token it signs and the prefix of `/.well-known/jwks.json`. A validator resolves the token's `iss` to that URL and fetches it, so a value nothing can reach means tokens issue and then fail to validate — the confusing way round. |
| `TOKEN_SIGNING_KEY_PEM` | Ed25519 PKCS#8 PEM. Unset generates one per process, which is fine for one instance and wrong for two: each would publish a different key, and a token issued by one would be refused by the other. Set it before scaling past one. |
| `ADMIN_TOKEN` | Enables `POST /admin/kill-switch`. Unset means the route is not mounted at all — a missing secret must never mean an open switch. |
| `OAUTH_STATE_KEY` | ≥32 bytes. Keeps encrypted OAuth state valid across restarts. |
| `TRUSTED_CLIENT_IP_HEADER` | The header the proxy in front **overwrites**. `cf-connecting-ip` behind Cloudflare. Empty falls back to `X-Forwarded-For`. |
| `CLIENT_IP_XFF_POSITION` | `rightmost` (default, safe) or `leftmost`. See below — do not set this from guesswork. |
| `<PROVIDER>_CLIENT_ID` / `_SECRET` | `GITHUB_`, `GOOGLE_`, `DISCORD_`. Absent credentials are not an error; the affected scenarios report themselves as not configured. |
| `GITHUB_KIT_CLIENT_ID` / `_SECRET` | A **second GitHub OAuth app**, separate from `GITHUB_CLIENT_ID` above, used only to push a generated project to a visitor's repo. Register it with the callback `<PUBLIC_BASE_URL>/api/github/callback` and no scope configured at the app level — the flow requests `public_repo` per authorisation. Keep it separate: the sign-in app should ask for the narrowest scope that proves identity, and merging the two would have every visitor trying the sign-in demo grant repo-write on the same click. Absent is not an error — the push reports itself unavailable and the zip download, which needs no GitHub account, is unaffected. |
| `<PROVIDER>_SITE_KEY` / `_SECRET_KEY` | `TURNSTILE_`, `HCAPTCHA_`, `RECAPTCHA_`. Both halves are required or the provider is not offered at all, because a site key without a secret renders a widget whose token nothing can spend — a failure at the last step with no clue as to why. Register the deployment's hostnames with each provider; a widget served from an unregistered host produces a token that fails verification and looks exactly like a bot. **`RECAPTCHA_SECRET_KEY` must be a legacy secret key** — see below. |

### reCAPTCHA and Enterprise

`authkestra-engine`'s `CaptchaVerifier` verifies reCAPTCHA by posting `secret`
and `response` to `www.google.com/recaptcha/api/siteverify`. That is the classic
protocol, and it is the only one the crate speaks.

Google has since moved reCAPTCHA's console into Google Cloud and steers new keys
towards **reCAPTCHA Enterprise**, which verifies through an assessment call to
`recaptchaenterprise.googleapis.com/v1/projects/{project}/assessments` — a
different endpoint, a different credential (a Cloud API key), a JSON request
body, and a response carrying a risk score rather than a bare `success` flag.
It is a different protocol, not a different key.

So there are two workable answers and one that is not:

* **Legacy secret key.** If the console will still issue one for a v2 checkbox
  key, put it in `RECAPTCHA_SECRET_KEY` and everything works as built.
* **Leave reCAPTCHA unconfigured.** Turnstile and hCaptcha both still verify the
  way the engine expects. The control only offers providers with keys, so an
  unset reCAPTCHA is invisible rather than broken — no code change.
* **Not this:** an Enterprise API key in `RECAPTCHA_SECRET_KEY`. `siteverify`
  will reject it, and the error reads like an invalid secret rather than like
  the wrong product.

Supporting Enterprise needs a new `CaptchaProvider` variant upstream in
`authkestra-engine`. Reimplementing the assessment call here instead would mean
the playground demonstrating the playground rather than the framework, and the
starter-kit fragment generating code that does not match what it claims.

## Settling the client-IP question

The rate limiter buckets by client IP, and which header carries it is a property
of the proxy in front. Both wrong answers are real:

- Read an entry the **client controls** → anyone bypasses rate limiting by
  sending the header themselves. This already happened once and was fixed.
- Read the **proxy's own** address → every visitor shares one bucket, so a
  handful of abusive requests throttle everybody. Coarse, but never a bypass.

The default is the second, safe case. Vendor documentation and reality do not
always agree, so **check rather than guess**. With `ADMIN_TOKEN` set:

```sh
curl -s -H "Authorization: Bearer $ADMIN_TOKEN" https://<api>/admin/client-ip | jq
```

It reports every candidate header that actually arrived, what each strategy
would select, and which one the limiter is using. Then set
`TRUSTED_CLIENT_IP_HEADER` (preferred) or `CLIENT_IP_XFF_POSITION` from the
answer.

For Render specifically: Render documents setting the first `X-Forwarded-For`
entry to the real client IP, which would make `leftmost` correct — but it also
fronts traffic with Cloudflare, so `cf-connecting-ip` may be present and is the
better source because Cloudflare strips client-supplied copies of it. The
endpoint settles which.

## Verifying a deployment

```sh
curl -s https://<api>/health
# {"status":"ok","version":"...","demo_enabled":true}

# CORS must echo the frontend origin, or the browser blocks everything:
curl -sD - -o /dev/null -H "Origin: https://<frontend>" https://<api>/api/session \
  | grep -i access-control-allow-origin
```

A green deploy is not proof of a working service — a process that exits at boot
still reports success, which is how a broken container shipped once. Both deploy
workflows smoke-test `/health` and fail the job if it never answers.
