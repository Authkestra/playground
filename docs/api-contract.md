# Playground API contract (v0)

Source of truth for `apps/api` handlers and `packages/api-types`. The TS types in
`packages/api-types` are **generated** from the Rust structs via `ts-rs`; this document
is the human-readable mirror. If they disagree, the Rust is right and the types need
regenerating (CI enforces this — see P0 "Mirror the framework's CI quality bar").

## Endpoints

| Method | Path                          | Body            | Response          |
| ------ | ----------------------------- | --------------- | ----------------- |
| GET    | `/health`                     | —               | `HealthResponse`  |
| GET    | `/api/session`                | —               | `DemoSessionView` |
| POST   | `/api/session/reset`          | —               | `DemoSessionView` |
| GET    | `/api/session/events`         | —               | `FlowEvent[]`     |
| GET    | `/api/scenarios`              | —               | `ScenarioSpec[]`  |
| POST   | `/api/scenarios/:id/configure`| `ConfigureBody` | `ConfigureResponse` |
| GET    | `/api/scenarios/:id/diff`     | —               | `ConfigDiff`      |
| POST   | `/api/scenarios/:id/try`      | `TryBody`       | `TryResult`       |
| POST   | `/api/scenarios/:id/action/:action` | scenario-specific | scenario-specific |

The demo session id travels in an HttpOnly cookie (`ak_demo`). Every endpoint under
`/api` lazily materialises a session if the cookie is absent or stale.

## The flow log

`GET /api/session/events` returns what the engine actually did for this visitor,
oldest first — a challenge issued, a signature verified, a counter advanced.
It is written for the visitor rather than for us: each entry names the step and
explains why it mattered. Server-side tracing stays separate.

`level` distinguishes outcomes that look similar but are not:

| Level | Meaning |
| --- | --- |
| `info` | Something progressed |
| `success` | A step completed |
| `rejected` | Refused for an ordinary reason — a wrong code, a cancelled prompt. **Not a fault**, and must not render like one. |
| `failed` | Something went wrong server-side |

`facts` is always present (possibly empty) and holds values worth showing
verbatim — a counter moving, an algorithm chosen. **Never secrets**; a test
asserts the TOTP secret never appears in it.

The log is capped per session and expires with it, so nothing has to clean it
up. `POST /api/session/reset` clears it.

## Ceremony actions

Registration and verification are multi-round-trip, which `configure`/`diff`/`try`
does not cover. Rather than giving each scenario its own routes — which would put
a per-scenario branch back into the HTTP layer — every step goes through one
generic endpoint:

```
POST /api/scenarios/:id/action/:action
```

A scenario advertises the steps it accepts in `ScenarioSpec.actions`, so the
frontend discovers them from data rather than hardcoding a list. Bodies and
responses are scenario-specific and typed in `packages/api-types`.

| Scenario | Action | Body | Response |
| --- | --- | --- | --- |
| `totp` | `provision` | `{}` | `TotpProvision` |
| `totp` | `verify` | `{ code }` | `TotpVerification` |
| `passkeys` | `register_start` | `{}` | WebAuthn `CredentialCreationOptions` |
| `passkeys` | `register_finish` | attestation | `PasskeyEnrolment` |
| `passkeys` | `authenticate_start` | `{}` | WebAuthn `CredentialRequestOptions` |
| `passkeys` | `authenticate_finish` | assertion | `PasskeyAuthResult` |

The `oauth` scenario has **no** actions, because OAuth is a navigation rather
than an XHR ceremony — see below.

Action endpoints share the tighter rate limit with `try`, since they create
credentials and reach third parties.

**A rejected credential is not an error.** `TotpVerification.verified: false` and
`PasskeyAuthResult.verified: false` are ordinary `200` responses describing a
normal outcome, and must render as results rather than failures. Genuine faults
use the error shapes below.

## Types

```ts
type ControlShape =
  | { kind: "toggle" }
  | { kind: "select_one"; options: ScenarioOption[] }
  | { kind: "select_many"; options: ScenarioOption[] };

interface ScenarioOption { id: string; label: string }

interface ScenarioSpec {
  id: string;
  name: string;
  summary: string;
  control: ControlShape;
  depends_on: string[];
  available: boolean;   // false when the kill switch has disabled this scenario
}

type ControlValue =
  | { kind: "toggle"; enabled: boolean }
  | { kind: "select_one"; selected: string | null }
  | { kind: "select_many"; selected: string[] };

interface DemoConfig { scenarios: Record<string, ControlValue> }

interface DemoSessionView {
  id: string;
  created_at: string;   // RFC3339
  expires_at: string;   // RFC3339
  config: DemoConfig;
}

type DiffKind = "added" | "removed" | "changed";

interface DiffEntry {
  kind: DiffKind;
  path: string;              // e.g. "scenarios.passkeys"
  before: string | null;
  after: string | null;
}

interface CrateRequirement { name: string; features: string[] }

interface Consequences {
  routes: string[];          // routes that appear/disappear in a real app
  requirements: string[];    // human-meaningful requirement changes
  crates: CrateRequirement[];
}

interface ConfigDiff { entries: DiffEntry[]; consequences: Consequences }

interface ConfigureBody { value: ControlValue }
interface ConfigureResponse { config: DemoConfig; diff: ConfigDiff }

interface TryBody { /* scenario-specific, opaque in v0 */ }
type TryOutcome = "ok" | "disabled" | "not_configured";
interface TryResult { outcome: TryOutcome; detail: string }

interface HealthResponse { status: string; version: string; demo_enabled: boolean }
```

## OAuth navigation routes

OAuth cannot use the action endpoint: the browser has to *leave* for the
provider and come back. Two ordinary GET routes handle it, and the frontend
starts the flow with a top-level navigation (`window.location.href = ...`), not
a fetch.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/auth/login/{provider}?mode=session\|jwt&scope=...` | Redirects to the provider. Generates PKCE and writes the encrypted `state` cookie. |
| GET | `/auth/callback/{provider}` | The provider's callback. Verifies state, exchanges the code, then redirects back to the frontend. |

`{provider}` is one of `github`, `google`, `discord`, and only providers with
credentials configured on the deployment are accepted — `ScenarioSpec.control`
for `oauth` lists exactly those, so the UI never offers a dead end.

`mode` selects how identity is established: `session` (default, server-side
session in a cookie) or `jwt` (stateless — nothing stored server-side, which is
the variant worth showing off).

**The callback always redirects back to the frontend**, never renders a page, so
the outcome arrives as query parameters the page reads:

| Parameter | Meaning |
| --- | --- |
| `oauth=success&provider=…` | Round trip completed |
| `oauth=denied&provider=…&reason=access_denied` | Visitor declined at the provider — an ordinary outcome, not an error |
| `oauth=error&provider=…&reason=…` | Could not complete — see the reasons below |

The `error` reasons are deliberately distinct, because they have unrelated fixes:

| Reason | Means | Usual cause |
| --- | --- | --- |
| `missing_code` | No `code`/`state` and no `error` either | Not a real callback |
| `unknown_provider` | No such provider registered | Credentials absent for it |
| `state_missing` | The `ak_state` cookie did not come back | More than 15 min elapsed, cookie blocked, or the flow started in another browser |
| `state_invalid` | The cookie came back but would not decrypt | `OAUTH_STATE_KEY` changed between the login and the callback |
| `exchange_failed` | State verified; the provider rejected the code exchange | Wrong client secret, or a `redirect_uri` that differs from the one registered |
| `callback_failed` | Anything else | Check the flow log |

Whatever the outcome, **the visitor's own flow log carries the underlying
message** (`GET /api/session/events`). Without it a failure here can only be
diagnosed from server logs.

The redirect target is the first entry of `ALLOWED_ORIGINS` and is never taken
from the request, so the callback cannot be turned into an open redirect.

### Registering the callback with a provider

The redirect URI to register is `{OAUTH_REDIRECT_BASE}/auth/callback/{provider}`
— note the order: `/auth/callback/github`, **not** `/auth/github/callback`.

## Errors

`429` from the rate limiter carries `{ "error": "rate_limited", "detail": "..." }`.
`503` with `{ "error": "demo_disabled" }` when the kill switch is off — the frontend
degrades to explainer-only mode rather than showing a broken control.
`404` `unknown_action` for a step the scenario does not define.
`410` `ceremony_expired` when a challenge timed out or was already answered —
the frontend should restart the ceremony rather than surfacing a dead end.
`400` `ceremony_rejected` when an authenticator's response fails verification.
