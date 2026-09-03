import type {
  ConfigDiff,
  ConfigureBody,
  ConfigureResponse,
  DemoSessionView,
  HealthResponse,
  ScenarioSpec,
  TryBody,
  TryResult,
} from "@playground/api-types";

export const API_BASE =
  process.env.NEXT_PUBLIC_API_BASE_URL ?? "http://localhost:8000";

export type ApiError =
  | { kind: "unavailable" } // network failure, DNS error, connection refused, ...
  | { kind: "demo_disabled" } // 503 { error: "demo_disabled" }
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

  if (res.status === 503) {
    return { ok: false, error: { kind: "demo_disabled" } };
  }

  if (res.status === 429) {
    const body = await safeJson(res);
    const detail =
      typeof body?.detail === "string"
        ? body.detail
        : "You're sending requests a bit too fast. Please slow down and try again shortly.";
    return { ok: false, error: { kind: "rate_limited", detail } };
  }

  if (!res.ok) {
    const body = await safeJson(res);
    const detail =
      (typeof body?.detail === "string" && body.detail) ||
      (typeof body?.error === "string" && body.error) ||
      res.statusText ||
      "Request failed";
    return { ok: false, error: { kind: "http_error", status: res.status, detail } };
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

export function configureScenario(
  id: string,
  body: ConfigureBody,
): Promise<ApiResult<ConfigureResponse>> {
  return request<ConfigureResponse>(
    `/api/scenarios/${encodeURIComponent(id)}/configure`,
    { method: "POST", body: JSON.stringify(body) },
  );
}

export function getScenarioDiff(id: string): Promise<ApiResult<ConfigDiff>> {
  return request<ConfigDiff>(`/api/scenarios/${encodeURIComponent(id)}/diff`);
}

export function tryScenario(
  id: string,
  body: TryBody = {},
): Promise<ApiResult<TryResult>> {
  return request<TryResult>(`/api/scenarios/${encodeURIComponent(id)}/try`, {
    method: "POST",
    body: JSON.stringify(body),
  });
}
