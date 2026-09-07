import { describe, it, expect } from "vitest";
import type { DemoConfig, ScenarioSpec } from "@playground/api-types";
import {
  PANEL_ORDER,
  hasSignInMethod,
  isScenarioLive,
  visiblePanels,
} from "./StepSignIn";

function spec(id: string, over: Partial<ScenarioSpec> = {}): ScenarioSpec {
  return {
    id,
    name: id,
    summary: `${id} summary`,
    control: { kind: "toggle" },
    depends_on: [],
    available: true,
    actions: [],
    unavailable_reason: null,
    ...over,
  };
}

/** Every scenario that has a panel, all available. */
const ALL: ScenarioSpec[] = PANEL_ORDER.map((id) => spec(id));

function on(...ids: string[]): DemoConfig {
  const scenarios: DemoConfig["scenarios"] = {};
  for (const id of PANEL_ORDER) {
    scenarios[id] =
      id === "oauth"
        ? { kind: "select_many", selected: ids.includes(id) ? ["github"] : [] }
        : { kind: "toggle", enabled: ids.includes(id) };
  }
  return { scenarios };
}

describe("visiblePanels", () => {
  // The regression. The old "is anything on?" check listed every panel except
  // the resource server, so turning on only the protected route rendered
  // "no sign-in method is turned on yet" and no panel at all — the scenario
  // looked like it did nothing.
  it("shows the protected route when it is the only thing switched on", () => {
    expect(visiblePanels(ALL, on("resource"), 0)).toEqual(["resource"]);
  });

  // Same shape, and the reason the bug was worth fixing at the root rather
  // than by adding one more boolean.
  it("shows bot protection when it is the only thing switched on", () => {
    expect(visiblePanels(ALL, on("captcha"), 0)).toEqual(["captcha"]);
  });

  it("shows nothing when nothing is switched on", () => {
    expect(visiblePanels(ALL, on(), 0)).toEqual([]);
  });

  it("orders panels by PANEL_ORDER, not by what the config happens to list", () => {
    const visible = visiblePanels(ALL, on("resource", "captcha", "totp"), 0);
    expect(visible).toEqual(["captcha", "totp", "resource"]);
  });

  // A captcha guards the form, so it has to come before the methods.
  it("puts bot protection in front of every sign-in method", () => {
    const visible = visiblePanels(ALL, on("captcha", "passkeys", "totp"), 0);
    expect(visible[0]).toBe("captcha");
  });

  // OAuth is selected but the deployment no longer offers that provider, so
  // there is no button to render — showing an empty block plus a divider would
  // be worse than showing nothing.
  it("hides OAuth when a selection has no offered providers behind it", () => {
    expect(visiblePanels(ALL, on("oauth"), 0)).toEqual([]);
    expect(visiblePanels(ALL, on("oauth"), 1)).toEqual(["oauth"]);
  });

  it("hides a panel the kill switch has disabled", () => {
    const killed = ALL.map((s) => (s.id === "totp" ? spec("totp", { available: false }) : s));
    expect(visiblePanels(killed, on("totp", "resource"), 0)).toEqual(["resource"]);
  });

  it("hides a scenario this deployment does not publish at all", () => {
    const without = ALL.filter((s) => s.id !== "captcha");
    expect(visiblePanels(without, on("captcha", "resource"), 0)).toEqual(["resource"]);
  });

  it("survives a config that predates a scenario", () => {
    // A session stored before captcha existed simply has no key for it.
    const older: DemoConfig = { scenarios: { resource: { kind: "toggle", enabled: true } } };
    expect(visiblePanels(ALL, older, 0)).toEqual(["resource"]);
    expect(visiblePanels(ALL, null, 0)).toEqual([]);
  });
});

describe("isScenarioLive", () => {
  it("requires both the visitor's choice and the deployment's availability", () => {
    expect(isScenarioLive(ALL, on("totp"), "totp")).toBe(true);
    expect(isScenarioLive(ALL, on(), "totp")).toBe(false);

    const killed = [spec("totp", { available: false })];
    expect(isScenarioLive(killed, on("totp"), "totp")).toBe(false);
  });

  it("is false for an id no scenario claims", () => {
    expect(isScenarioLive(ALL, on("totp"), "nonsense")).toBe(false);
  });
});

describe("hasSignInMethod", () => {
  // The heading must not tell someone to "choose how to continue" when the
  // only thing they turned on was a protected route or a captcha.
  it("is false when only non-sign-in panels are showing", () => {
    expect(hasSignInMethod(["resource"])).toBe(false);
    expect(hasSignInMethod(["captcha", "resource"])).toBe(false);
    expect(hasSignInMethod([])).toBe(false);
  });

  it("is true as soon as a real sign-in method is showing", () => {
    for (const id of ["oauth", "passkeys", "totp"]) {
      expect(hasSignInMethod(["captcha", id])).toBe(true);
    }
  });
});

describe("PANEL_ORDER", () => {
  // A scenario with a tester but no entry here renders nothing, which is the
  // failure this whole file exists to catch.
  it("covers every scenario that has a flow tester", () => {
    for (const id of ["captcha", "oauth", "passkeys", "totp", "resource"]) {
      expect(PANEL_ORDER).toContain(id);
    }
  });

  it("lists each panel once", () => {
    expect(new Set(PANEL_ORDER).size).toBe(PANEL_ORDER.length);
  });
});
