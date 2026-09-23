import { describe, it, expect } from "vitest";
import {
  FIXED_CHOICE_LABELS,
  FORGERIES,
  LOCAL_VERDICT_LABELS,
  RUN_STEP_LABELS,
  VERDICT_LABELS,
  localVerdictStyle,
  shouldRefetchKeys,
  verdictStyle,
} from "./ResourcePanel";
import type { LocalVerdict } from "@/lib/jwt";

/**
 * Every verdict the API can return, from `TokenVerdict` in
 * apps/api/src/scenario/resource.rs. A verdict with no label renders as its
 * raw wire name, which is the kind of thing nobody notices in review.
 */
const BACKEND_VERDICTS = [
  "accepted",
  "absent",
  "malformed",
  "missing_kid",
  "unknown_kid",
  "untrusted_issuer",
  "wrong_issuer",
  "wrong_audience",
  "expired",
  "bad_signature",
  "keys_unreachable",
  "rejected",
];

/** Every forgery the API can mint, from `Forgery` in the same file. */
const BACKEND_FORGERIES = [
  "unknown_kid",
  "bad_signature",
  "untrusted_issuer",
  "wrong_audience",
  "expired",
  "missing_kid",
];

describe("VERDICT_LABELS", () => {
  it("labels every verdict the API can return", () => {
    for (const verdict of BACKEND_VERDICTS) {
      expect(VERDICT_LABELS[verdict], `no label for ${verdict}`).toBeDefined();
    }
  });
});

describe("FORGERIES", () => {
  it("offers every forgery the API can mint", () => {
    expect(FORGERIES.map((f) => f.kind).sort()).toEqual([...BACKEND_FORGERIES].sort());
  });

  // These are now the six "intentionally broken" `<option>`s in one native
  // `<select>` rather than six buttons, but a label still has to be distinct
  // or two forgeries would be indistinguishable in the dropdown.
  it("gives each one a distinct label", () => {
    const labels = FORGERIES.map((f) => f.label);
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe("FIXED_CHOICE_LABELS", () => {
  // The "Try:" select offers these three alongside the six forgeries; they
  // must read as distinct from every forgery label too, or e.g. "No token"
  // could be confused for one of the broken-on-purpose options.
  it("is distinct from every forgery label", () => {
    const forgeryLabels = new Set(FORGERIES.map((f) => f.label));
    expect(forgeryLabels.has(FIXED_CHOICE_LABELS.valid)).toBe(false);
    expect(forgeryLabels.has(FIXED_CHOICE_LABELS.none)).toBe(false);
    expect(forgeryLabels.has(FIXED_CHOICE_LABELS.custom)).toBe(false);
  });

  it("gives all three fixed choices distinct labels", () => {
    const labels = Object.values(FIXED_CHOICE_LABELS);
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe("RUN_STEP_LABELS", () => {
  // The one-click run still does these three things in this order; the
  // checklist is what keeps the fetch-then-verify separation from #76
  // visible now that nothing gates it behind a second click.
  it("names exactly the three steps a run performs, in order", () => {
    expect(Object.keys(RUN_STEP_LABELS)).toEqual(["call", "fetchKeys", "verify"]);
  });

  it("gives each step a distinct, non-empty label", () => {
    const labels = Object.values(RUN_STEP_LABELS);
    expect(new Set(labels).size).toBe(labels.length);
    for (const label of labels) expect(label.length).toBeGreaterThan(0);
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

  it("refetches when the run's issuer differs from what's cached", () => {
    expect(shouldRefetchKeys("https://a.example/jwks.json", "https://b.example/jwks.json")).toBe(
      true,
    );
  });
});

describe("verdictStyle", () => {
  // A 401 is the ordinary answer here, so destructive has to mean more than
  // "refused" or it means nothing.
  it("reserves destructive for a forgery and for a broken validator", () => {
    for (const verdict of ["bad_signature", "keys_unreachable"]) {
      expect(verdictStyle(verdict)).toContain("destructive");
    }
  });

  it("treats an ordinary rejection as warning, not destructive", () => {
    for (const verdict of ["expired", "wrong_audience", "unknown_kid", "untrusted_issuer"]) {
      expect(verdictStyle(verdict)).toContain("warning");
      expect(verdictStyle(verdict)).not.toContain("destructive");
    }
  });

  it("styles acceptance as success and an absent token as neutral", () => {
    expect(verdictStyle("accepted")).toContain("success");
    expect(verdictStyle("absent")).toContain("muted");
  });

  // A verdict added upstream must still render as *something* legible.
  it("falls back rather than returning nothing for an unknown verdict", () => {
    expect(verdictStyle("something_new_upstream")).toContain("warning");
  });
});

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

  // The local and remote answers are meant to be compared, which only works
  // if the same outcome is called the same thing on both sides.
  it("uses the API's own words where both sides answer the same question", () => {
    expect(LOCAL_VERDICT_LABELS.unknown_kid).toBe(VERDICT_LABELS.unknown_kid);
    expect(LOCAL_VERDICT_LABELS.bad_signature).toBe(VERDICT_LABELS.bad_signature);
    expect(LOCAL_VERDICT_LABELS.missing_kid).toBe(VERDICT_LABELS.missing_kid);
    expect(LOCAL_VERDICT_LABELS.malformed).toBe(VERDICT_LABELS.malformed);
  });
});

describe("localVerdictStyle", () => {
  it("reserves destructive for a signature that did not verify", () => {
    expect(localVerdictStyle("bad_signature")).toContain("destructive");
    for (const kind of LOCAL_VERDICTS.filter((k) => k !== "bad_signature")) {
      expect(localVerdictStyle(kind), kind).not.toContain("destructive");
    }
  });

  it("styles a local verification as success", () => {
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
