import type {
  ConfigDiff,
  ConfigureBody,
  ConfigureResponse,
  DemoSessionView,
  FlowEvent,
  HealthResponse,
  ScenarioSpec,
} from "@playground/api-types";

export const API_BASE =
  process.env.NEXT_PUBLIC_API_BASE_URL ?? "http://localhost:8000";

export type ApiError =
  | { kind: "unavailable" } // network failure, DNS error, connection refused, ...
  | { kind: "demo_disabled" } // 503 { error: "demo_disabled" }
  | { kind: "state_unavailable"; detail: string } // 503 { error: "state_unavailable" }
  | { kind: "rate_limited"; detail: string } // 429 { error: "rate_limited", detail }
  | { kind: "http_error"; status: number; detail: string };

export type ApiResult<T> =
  | { ok: true; data: T }
  | { ok: false; error: ApiError };

async function request<T>(
  path: string,
  init?: RequestInit,
): Promise<ApiResult<T>> {
  let res: Response;
  try {
    res = await fetch(`${API_BASE}${path}`, {
      ...init,
      credentials: "include",
      headers: {
        "Content-Type": "application/json",
        ...(init?.headers ?? {}),
      },
    });
  } catch {
    // Network failure — API is unreachable (backend not running, CORS, DNS, ...).
    return { ok: false, error: { kind: "unavailable" } };
  }

  if (!res.ok) {
    const error = await handleErrorResponse(res);
    return { ok: false, error };
  }

  const data = await safeJson(res);
  if (data === null) {
    return {
      ok: false,
      error: { kind: "http_error", status: res.status, detail: "Malformed response body" },
    };
  }

  return { ok: true, data: data as T };
}

async function safeJson(res: Response): Promise<any | null> {
  try {
    return await res.json();
  } catch {
    return null;
  }
}

/**
 * Handle non-ok HTTP responses: check status codes and parse response bodies
 * to produce an ApiError. This factors out the shared 503/429/!ok logic that
 * would otherwise be duplicated across request handlers.
 */
async function handleErrorResponse(res: Response): Promise<ApiError> {
  const body = await safeJson(res);

  // 503 can mean two different things: the demo is switched off, or the state
  // backend is unreachable. Read the error field to discriminate.
  if (res.status === 503) {
    const error = typeof body?.error === "string" ? body.error : "";
    if (error === "state_unavailable") {
      const detail =
        typeof body?.detail === "string"
          ? body.detail
          : "The playground's state store is unreachable. This is temporary.";
      return { kind: "state_unavailable", detail };
    }
    if (error === "demo_disabled") {
      return { kind: "demo_disabled" };
    }
    // Unrecognized 503 (neither demo_disabled nor state_unavailable) falls
    // through to be treated as a generic http_error.
  }

  // 429: rate limited.
  if (res.status === 429) {
    const detail =
      typeof body?.detail === "string"
        ? body.detail
        : "You're sending requests a bit too fast. Please slow down and try again shortly.";
    return { kind: "rate_limited", detail };
  }

  // Other HTTP errors (4xx, 5xx).
  const detail =
    (typeof body?.detail === "string" && body.detail) ||
    (typeof body?.error === "string" && body.error) ||
    res.statusText ||
    "Request failed";
  return { kind: "http_error", status: res.status, detail };
}

export function getHealth(): Promise<ApiResult<HealthResponse>> {
  return request<HealthResponse>("/health");
}

export function getSession(): Promise<ApiResult<DemoSessionView>> {
  return request<DemoSessionView>("/api/session");
}

export function resetSession(): Promise<ApiResult<DemoSessionView>> {
  return request<DemoSessionView>("/api/session/reset", { method: "POST" });
}

export function getScenarios(): Promise<ApiResult<ScenarioSpec[]>> {
  return request<ScenarioSpec[]>("/api/scenarios");
}

/**
 * The flow log: what the engine actually did for this visitor, oldest first.
 * See docs/api-contract.md#the-flow-log.
 */
export function getSessionEvents(): Promise<ApiResult<FlowEvent[]>> {
  return request<FlowEvent[]>("/api/session/events");
}

export function configureScenario(
  id: string,
  body: ConfigureBody,
): Promise<ApiResult<ConfigureResponse>> {
  return request<ConfigureResponse>(
    `/api/scenarios/${encodeURIComponent(id)}/configure`,
    { method: "POST", body: JSON.stringify(body) },
  );
}

/**
 * Generic ceremony-step endpoint: `POST /api/scenarios/:id/action/:action`.
 * Scenarios advertise the steps they accept via `ScenarioSpec.actions`
 * (e.g. `totp` accepts `"provision"` and `"verify"`).
 */
export function scenarioAction<T>(
  id: string,
  action: string,
  body: unknown = {},
): Promise<ApiResult<T>> {
  return request<T>(
    `/api/scenarios/${encodeURIComponent(id)}/action/${encodeURIComponent(action)}`,
    { method: "POST", body: JSON.stringify(body) },
  );
}

/**
 * The generated project, as a zip.
 *
 * Deliberately not routed through `request`, which assumes a JSON body. The
 * error shapes are shared, so a rate-limited download reads the same as a
 * rate-limited anything else.
 */
export type StarterKitDownload = { blob: Blob; filename: string };

const FALLBACK_FILENAME = "authkestra-starter.zip";

export async function downloadStarterKit(): Promise<ApiResult<StarterKitDownload>> {
  let res: Response;
  try {
    res = await fetch(`${API_BASE}/api/starter-kit`, { credentials: "include" });
  } catch {
    return { ok: false, error: { kind: "unavailable" } };
  }

  if (!res.ok) {
    const error = await handleErrorResponse(res);
    return { ok: false, error };
  }

  return { ok: true, data: { blob: await res.blob(), filename: filenameFrom(res) } };
}

/**
 * The name the server chose. Readable only because the API exposes
 * `Content-Disposition` through CORS; if that ever stops being true the
 * download still works, just under a generic name.
 */
function filenameFrom(res: Response): string {
  const header = res.headers.get("content-disposition");
  if (!header) return FALLBACK_FILENAME;

  const match = /filename="?([^";]+)"?/i.exec(header);
  const name = match?.[1]?.trim();
  if (!name) return FALLBACK_FILENAME;

  // The server asserts its names need no quoting or escaping. Anything else
  // did not come from it, and is not worth handing to a file save.
  return /^[A-Za-z0-9._-]+$/.test(name) ? name : FALLBACK_FILENAME;
}
