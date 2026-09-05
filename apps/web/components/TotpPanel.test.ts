import { describe, it, expect } from "vitest";
import { normalizeCode } from "./TotpPanel";

describe("normalizeCode", () => {
  it("accepts spaces as visual separators and strips them", () => {
    expect(normalizeCode("123 456")).toBe("123456");
  });

  it("accepts dashes as visual separators and strips them", () => {
    expect(normalizeCode("123-456")).toBe("123456");
  });

  it("accepts both spaces and dashes together", () => {
    expect(normalizeCode("123 456-789")).toBe("123456");
  });

  it("drops non-digit characters", () => {
    expect(normalizeCode("123abc456")).toBe("123456");
    expect(normalizeCode("1a2b3c")).toBe("123");
  });

  it("caps output at six characters", () => {
    expect(normalizeCode("1234567")).toBe("123456");
    expect(normalizeCode("12345678901")).toBe("123456");
  });

  it("handles standard TOTP input format correctly", () => {
    expect(normalizeCode("123456")).toBe("123456");
  });

  it("handles input with both spaces and trailing characters", () => {
    expect(normalizeCode("123 456abc")).toBe("123456");
  });

  it("handles empty input", () => {
    expect(normalizeCode("")).toBe("");
  });

  it("handles input with only spaces and dashes", () => {
    expect(normalizeCode("   ---   ")).toBe("");
  });

  it("handles input with only letters", () => {
    expect(normalizeCode("abc")).toBe("");
  });

  it("handles long input with mixed separators", () => {
    expect(normalizeCode("123 456 789-012-345")).toBe("123456");
  });
});
