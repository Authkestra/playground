import { describe, it, expect } from "vitest";
import type { DemoConfig } from "@playground/api-types";
import { isScenarioActive } from "./FlowTopology";

describe("FlowTopology", () => {
  it("determines scenario active state accurately", () => {
    const config: DemoConfig = {
      scenarios: {
        mcp: { kind: "select_one", selected: "human_approved" },
        policy: { kind: "select_one", selected: "department_abac" },
        totp: { kind: "toggle", enabled: false },
        passwordless: { kind: "select_many", selected: ["magic_link", "otp"] },
      },
    };

    expect(isScenarioActive(config, "mcp")).toBe(true);
    expect(isScenarioActive(config, "policy")).toBe(true);
    expect(isScenarioActive(config, "totp")).toBe(false);
    expect(isScenarioActive(config, "passwordless")).toBe(true);
    expect(isScenarioActive(config, "unknown")).toBe(false);
    expect(isScenarioActive(null, "mcp")).toBe(false);
  });
});
