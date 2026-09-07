import { describe, it, expect } from "vitest";
import { FORGERIES, VERDICT_LABELS, decodeHeader, verdictStyle } from "./ResourcePanel";

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

  it("gives each one a distinct button label", () => {
    const labels = FORGERIES.map((f) => f.label);
    expect(new Set(labels).size).toBe(labels.length);
  });
});

describe("verdictStyle", () => {
  // A 401 is the ordinary answer here, so red has to mean more than "refused"
  // or it means nothing.
  it("reserves red for a forgery and for a broken validator", () => {
    for (const verdict of ["bad_signature", "keys_unreachable"]) {
      expect(verdictStyle(verdict)).toContain("rose");
    }
  });

  it("treats an ordinary rejection as amber, not red", () => {
    for (const verdict of ["expired", "wrong_audience", "unknown_kid", "untrusted_issuer"]) {
      expect(verdictStyle(verdict)).toContain("amber");
      expect(verdictStyle(verdict)).not.toContain("rose");
    }
  });

  it("styles acceptance as emerald and an absent token as neutral", () => {
    expect(verdictStyle("accepted")).toContain("emerald");
    expect(verdictStyle("absent")).toContain("slate");
  });

  // A verdict added upstream must still render as *something* legible.
  it("falls back rather than returning nothing for an unknown verdict", () => {
    expect(verdictStyle("something_new_upstream")).toContain("amber");
  });
});

describe("decodeHeader", () => {
  // The point is that the `kid` shown next to the token is read out of the
  // token, not taken from a field the server also sent.
  it("reads the kid out of a real token header", () => {
    const header = Buffer.from(JSON.stringify({ alg: "EdDSA", kid: "key-1" })).toString(
      "base64url",
    );
    const decoded = decodeHeader(`${header}.payload.signature`);
    expect(decoded).toContain("key-1");
    expect(decoded).toContain("EdDSA");
  });

  it("returns null for anything it cannot read, rather than throwing", () => {
    for (const junk of ["", "not-base64!!", "....", "hello"]) {
      expect(() => decodeHeader(junk)).not.toThrow();
    }
    expect(decodeHeader("hello")).toBeNull();
  });
});
