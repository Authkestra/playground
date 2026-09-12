import { describe, it, expect } from "vitest";
import type { ApiError } from "@/lib/api";
import {
  defaultRepoName,
  describeGithubConnectReason,
  describeGithubPushError,
  isValidRepoName,
} from "./StepDownload";

describe("isValidRepoName", () => {
  it("accepts letters, digits, hyphens, underscores and periods", () => {
    expect(isValidRepoName("my-repo_2.0")).toBe(true);
    expect(isValidRepoName("authkestra-starter")).toBe(true);
    expect(isValidRepoName("A1")).toBe(true);
  });

  it("rejects an empty name", () => {
    expect(isValidRepoName("")).toBe(false);
  });

  it("rejects names GitHub reserves", () => {
    expect(isValidRepoName(".")).toBe(false);
    expect(isValidRepoName("..")).toBe(false);
  });

  it("rejects a name longer than 100 characters", () => {
    expect(isValidRepoName("a".repeat(100))).toBe(true);
    expect(isValidRepoName("a".repeat(101))).toBe(false);
  });

  it("rejects characters outside GitHub's own set", () => {
    expect(isValidRepoName("my repo")).toBe(false);
    expect(isValidRepoName("repo/name")).toBe(false);
    expect(isValidRepoName("repo@name")).toBe(false);
    expect(isValidRepoName("répo")).toBe(false);
  });
});

describe("defaultRepoName", () => {
  it("falls back to a plain name when nothing is turned on", () => {
    expect(defaultRepoName([])).toBe("authkestra-starter");
  });

  it("folds in the included scenario ids so two configurations differ", () => {
    expect(defaultRepoName(["passkeys"])).toBe("authkestra-starter-passkeys");
    expect(defaultRepoName(["passkeys", "totp"])).toBe("authkestra-starter-passkeys-totp");
  });
});

describe("describeGithubPushError", () => {
  function err(kind: ApiError["kind"], extra: Record<string, unknown> = {}): ApiError {
    return { kind, ...extra } as ApiError;
  }

  // The point of this whole mapping: none of these collapse into one
  // generic "something went wrong" message, and each names its own fix.
  it("gives github_push_not_configured a deployment-limitation message that points at the zip", () => {
    const message = describeGithubPushError(err("github_push_not_configured"));
    expect(message).toMatch(/hasn't set up GitHub push/i);
    expect(message).toMatch(/zip/i);
  });

  it("tells the visitor to pick another name when it's taken", () => {
    expect(describeGithubPushError(err("github_repo_name_taken"))).toMatch(/already exists/i);
  });

  it("surfaces GitHub's own message for an invalid repo name verbatim", () => {
    const message = describeGithubPushError(
      err("github_invalid_repo_name", { detail: "`bad name` is not a repository name GitHub will accept." }),
    );
    expect(message).toBe("`bad name` is not a repository name GitHub will accept.");
  });

  it("offers to reconnect when the token was rejected", () => {
    expect(describeGithubPushError(err("github_token_rejected"))).toMatch(/reconnect/i);
  });

  it("offers to reconnect when a scope is missing", () => {
    expect(describeGithubPushError(err("github_scope_missing"))).toMatch(/reconnect/i);
  });

  it("suggests waiting when GitHub itself is rate limiting", () => {
    const message = describeGithubPushError(
      err("github_rate_limited", { detail: "GitHub's own rate limit was hit. Wait a few minutes and try again." }),
    );
    expect(message).toMatch(/wait/i);
  });

  it("suggests retrying on a network error reaching GitHub", () => {
    const message = describeGithubPushError(
      err("github_network_error", { detail: "Could not reach GitHub: timed out." }),
    );
    expect(message).toBe("Could not reach GitHub: timed out.");
  });

  it("tells the visitor to reconnect when the connection is simply gone", () => {
    expect(describeGithubPushError(err("github_not_connected"))).toMatch(/connect/i);
  });

  it("keeps distinct messages across every kind — no two collapse to the same text", () => {
    const kinds: ApiError["kind"][] = [
      "github_push_not_configured",
      "github_not_connected",
      "github_repo_name_taken",
      "github_token_rejected",
      "github_scope_missing",
    ];
    const messages = kinds.map((kind) => describeGithubPushError(err(kind)));
    expect(new Set(messages).size).toBe(messages.length);
  });
});

describe("describeGithubConnectReason", () => {
  it("names deployment limitations plainly", () => {
    expect(describeGithubConnectReason("not_configured")).toMatch(/hasn't set up GitHub push/i);
  });

  it("falls back to the raw reason for something unrecognised", () => {
    expect(describeGithubConnectReason("a_future_reason")).toBe("a_future_reason");
  });
});
