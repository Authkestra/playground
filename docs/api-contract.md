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

The demo session id travels in an HttpOnly cookie (`ak_demo`). Every endpoint under
`/api` lazily materialises a session if the cookie is absent or stale.

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
