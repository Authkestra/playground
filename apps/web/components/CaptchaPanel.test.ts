import { describe, it, expect } from "vitest";
import { PROVIDER_SCRIPTS, verdictLabel, verdictStyle } from "./CaptchaPanel";

// Every provider id the backend's KNOWN_PROVIDERS can hand back
// (apps/api/src/scenario/captcha.rs). A missing entry here would silently
// render nothing for that provider's widget.
const BACKEND_PROVIDER_IDS = ["turnstile", "hcaptcha", "recaptcha"];

describe("PROVIDER_SCRIPTS", () => {
  it("has a script entry for every provider id the backend can return", () => {
    for (const id of BACKEND_PROVIDER_IDS) {
      expect(PROVIDER_SCRIPTS[id]).toBeDefined();
    }
  });

  it("gives each entry a script URL and a distinct global", () => {
    const globals = new Set<string>();
    for (const id of BACKEND_PROVIDER_IDS) {
      const entry = PROVIDER_SCRIPTS[id];
      expect(entry.src).toMatch(/^https:\/\//);
      globals.add(entry.global);
    }
    expect(globals.size).toBe(BACKEND_PROVIDER_IDS.length);
  });
});

describe("verdictStyle", () => {
  it("styles a verified token as success", () => {
    expect(verdictStyle(true)).toContain("success");
  });

  it("styles a not-verified token as warning, not destructive — a failed captcha is expected here", () => {
    expect(verdictStyle(false)).toContain("warning");
    expect(verdictStyle(false)).not.toContain("destructive");
  });
});

describe("verdictLabel", () => {
  it("labels true as Verified", () => {
    expect(verdictLabel(true)).toBe("Verified");
  });

  it("labels false as Not verified", () => {
    expect(verdictLabel(false)).toBe("Not verified");
  });
});
