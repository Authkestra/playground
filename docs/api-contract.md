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
| GET    | `/api/scenarios`              | —               | `ScenarioSpec[]`  |
| POST   | `/api/scenarios/:id/configure`| `ConfigureBody` | `ConfigureResponse` |
| GET    | `/api/scenarios/:id/diff`     | —               | `ConfigDiff`      |
| POST   | `/api/scenarios/:id/try`      | `TryBody`       | `TryResult`       |
| POST   | `/api/scenarios/:id/action/:action` | scenario-specific | scenario-specific |

The demo session id travels in an HttpOnly cookie (`ak_demo`). Every endpoint under
`/api` lazily materialises a session if the cookie is absent or stale.

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

## Errors

`429` from the rate limiter carries `{ "error": "rate_limited", "detail": "..." }`.
`503` with `{ "error": "demo_disabled" }` when the kill switch is off — the frontend
degrades to explainer-only mode rather than showing a broken control.
`404` `unknown_action` for a step the scenario does not define.
`410` `ceremony_expired` when a challenge timed out or was already answered —
the frontend should restart the ceremony rather than surfacing a dead end.
`400` `ceremony_rejected` when an authenticator's response fails verification.
