import { describe, it, expect } from "vitest";
import { LOCAL_VERDICT_LABELS, localVerdictStyle, shouldRefetchKeys } from "./ResourcePanel";
import type { LocalVerdict } from "@/lib/jwt";

/**
 * Every verdict the browser can reach on its own, from `LocalVerdict` in
 * apps/web/lib/jwt.ts. Decoding and verification are tested there, against
 * real signatures; what is tested here is that none of their outcomes can
 * reach the panel without a name and a colour.
 */
const LOCAL_VERDICTS: LocalVerdict["kind"][] = [
  "verified",
  "bad_signature",
  "unknown_kid",
  "missing_kid",
  "malformed",
  "unsupported_alg",
  "unsupported",
];

describe("LOCAL_VERDICT_LABELS", () => {
  it("labels every verdict the browser can reach", () => {
    for (const kind of LOCAL_VERDICTS) {
      expect(LOCAL_VERDICT_LABELS[kind], `no label for ${kind}`).toBeDefined();
    }
  });

  it("gives every verdict a distinct, non-empty label", () => {
    const labels = Object.values(LOCAL_VERDICT_LABELS);
    expect(new Set(labels).size).toBe(labels.length);
    for (const label of labels) expect(label.length).toBeGreaterThan(0);
  });
});

describe("localVerdictStyle", () => {
  it("reserves destructive for a signature that did not verify", () => {
    expect(localVerdictStyle("bad_signature")).toContain("destructive");
    for (const kind of LOCAL_VERDICTS.filter((k) => k !== "bad_signature")) {
      expect(localVerdictStyle(kind), kind).not.toContain("destructive");
    }
  });

  it("styles a verification as success", () => {
    expect(localVerdictStyle("verified")).toContain("success");
  });

  // "This browser cannot check it" is a fact about the browser, not a finding
  // about the token, and colouring it as a failure would say otherwise.
  it("keeps an uncheckable token neutral rather than alarming", () => {
    expect(localVerdictStyle("unsupported")).toContain("muted");
    expect(localVerdictStyle("unsupported_alg")).toContain("muted");
  });

  it("gives every verdict some colour rather than none", () => {
    for (const kind of LOCAL_VERDICTS) {
      expect(localVerdictStyle(kind), kind).toContain("border-");
    }
  });
});

describe("shouldRefetchKeys", () => {
  it("refetches when nothing is cached yet", () => {
    expect(shouldRefetchKeys(null, "https://issuer.example/jwks.json")).toBe(true);
  });

  it("reuses the cache when the target URL matches what's cached", () => {
    const url = "https://issuer.example/jwks.json";
    expect(shouldRefetchKeys(url, url)).toBe(false);
  });

  it("refetches when the field's URL differs from what's cached", () => {
    expect(shouldRefetchKeys("https://a.example/jwks.json", "https://b.example/jwks.json")).toBe(
      true,
    );
  });
});
