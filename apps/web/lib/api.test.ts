import { describe, it, expect } from "vitest";
import { errorFromBody, serializeDeployTargets } from "./api";

describe("serializeDeployTargets", () => {
  it("omits the parameter entirely when the visitor never touched the control", () => {
    expect(serializeDeployTargets(undefined)).toBeUndefined();
  });

  // The distinction the task is built around: an explicit "nothing selected"
  // is a real choice — someone deploying by hand — and must still produce a
  // value the server can see, never fall back to being indistinguishable
  // from an absent parameter.
  it("serialises an explicit empty selection as an empty string, not undefined", () => {
    const result = serializeDeployTargets({});
    expect(result).toBe("");
    expect(result).not.toBeUndefined();
  });

  it("serialises an explicit selection with everything turned off the same way", () => {
    expect(serializeDeployTargets({ docker: false, render: false, fly: false, railway: false })).toBe("");
  });

  it("lists exactly what was selected, in a stable order", () => {
    expect(serializeDeployTargets({ docker: true })).toBe("docker");
    expect(serializeDeployTargets({ render: true, railway: true })).toBe("render,railway");
    expect(serializeDeployTargets({ docker: true, render: true, fly: true, railway: true })).toBe(
      "docker,render,fly,railway",
    );
  });

  it("ignores a field explicitly set to false alongside others set to true", () => {
    expect(serializeDeployTargets({ docker: false, render: true })).toBe("render");
  });
});

describe("errorFromBody", () => {
  it("maps demo_disabled regardless of the exact status", () => {
    expect(errorFromBody(503, "Service Unavailable", { error: "demo_disabled" })).toEqual({
      kind: "demo_disabled",
    });
  });

  it("maps state_unavailable and carries the server's detail", () => {
    expect(
      errorFromBody(503, "Service Unavailable", { error: "state_unavailable", detail: "kv store is down" }),
    ).toEqual({ kind: "state_unavailable", detail: "kv store is down" });
  });

  it("falls back to a default detail when state_unavailable carries none", () => {
    const result = errorFromBody(503, "Service Unavailable", { error: "state_unavailable" });
    expect(result.kind).toBe("state_unavailable");
    expect((result as { detail: string }).detail.length).toBeGreaterThan(0);
  });

  it("maps a plain 429 with no error code to rate_limited", () => {
    expect(errorFromBody(429, "Too Many Requests", {})).toEqual({
      kind: "rate_limited",
      detail: expect.any(String),
    });
  });

  // Each of #40's GitHub-push error codes gets its own ApiError kind rather
  // than collapsing into http_error — the whole point of the mapping.
  it("maps every github_push error code onto its own kind", () => {
    expect(errorFromBody(503, "", { error: "github_push_not_configured" })).toEqual({
      kind: "github_push_not_configured",
    });
    expect(errorFromBody(400, "", { error: "github_not_connected" })).toEqual({
      kind: "github_not_connected",
    });
    expect(errorFromBody(409, "", { error: "github_repo_name_taken" })).toEqual({
      kind: "github_repo_name_taken",
    });
    expect(errorFromBody(401, "", { error: "github_token_rejected" })).toEqual({
      kind: "github_token_rejected",
    });
    expect(errorFromBody(403, "", { error: "github_scope_missing" })).toEqual({
      kind: "github_scope_missing",
    });
  });

  it("carries the backend's detail through for the codes that vary", () => {
    expect(
      errorFromBody(400, "", { error: "github_invalid_repo_name", detail: "`bad name` is invalid." }),
    ).toEqual({ kind: "github_invalid_repo_name", detail: "`bad name` is invalid." });
    expect(
      errorFromBody(429, "", { error: "github_rate_limited", detail: "wait a few minutes" }),
    ).toEqual({ kind: "github_rate_limited", detail: "wait a few minutes" });
    expect(
      errorFromBody(502, "", { error: "github_network_error", detail: "timed out" }),
    ).toEqual({ kind: "github_network_error", detail: "timed out" });
  });

  it("falls back to a generic http_error for anything this vocabulary doesn't name", () => {
    expect(errorFromBody(418, "I'm a teapot", { error: "teapot_mode" })).toEqual({
      kind: "http_error",
      status: 418,
      detail: "teapot_mode",
    });
  });

  it("prefers detail, then the error code, then statusText, for the generic fallback", () => {
    expect(errorFromBody(500, "Internal Server Error", {})).toEqual({
      kind: "http_error",
      status: 500,
      detail: "Internal Server Error",
    });
    expect(errorFromBody(500, "", {})).toEqual({
      kind: "http_error",
      status: 500,
      detail: "Request failed",
    });
  });

  it("tolerates a body that isn't an object at all", () => {
    expect(errorFromBody(500, "Internal Server Error", null)).toEqual({
      kind: "http_error",
      status: 500,
      detail: "Internal Server Error",
    });
  });
});
