import { describe, it, expect } from "vitest";
import { readOAuthReturn } from "./oauth";

describe("readOAuthReturn", () => {
  describe("non-OAuth queries", () => {
    it("returns null when query string is empty", () => {
      expect(readOAuthReturn("")).toBeNull();
    });

    it("returns null when oauth parameter is missing", () => {
      expect(readOAuthReturn("?foo=bar&baz=qux")).toBeNull();
    });

    it("returns null when provider parameter is missing but oauth is present", () => {
      expect(readOAuthReturn("?oauth=success&mode=session")).toBeNull();
    });

    it("returns null when both oauth and provider are missing", () => {
      expect(readOAuthReturn("?mode=session&reason=something")).toBeNull();
    });
  });

  describe("success with mode=session", () => {
    it("returns success status with mode session", () => {
      const result = readOAuthReturn("?oauth=success&provider=google&mode=session");
      expect(result).toEqual({
        status: "success",
        provider: "google",
        mode: "session",
      });
    });
  });

  describe("success with mode=jwt", () => {
    it("returns success status with mode jwt", () => {
      const result = readOAuthReturn("?oauth=success&provider=github&mode=jwt");
      expect(result).toEqual({
        status: "success",
        provider: "github",
        mode: "jwt",
      });
    });
  });

  describe("success with missing or bogus mode", () => {
    it("returns success with null mode when mode parameter is missing", () => {
      const result = readOAuthReturn("?oauth=success&provider=google");
      expect(result).toEqual({
        status: "success",
        provider: "google",
        mode: null,
      });
    });

    it("returns success with null mode when mode is bogus", () => {
      const result = readOAuthReturn("?oauth=success&provider=google&mode=invalid");
      expect(result).toEqual({
        status: "success",
        provider: "google",
        mode: null,
      });
    });

    it("returns success with null mode for unexpected mode values", () => {
      expect(readOAuthReturn("?oauth=success&provider=google&mode=Session")).toEqual({
        status: "success",
        provider: "google",
        mode: null,
      });
      expect(readOAuthReturn("?oauth=success&provider=google&mode=JWT")).toEqual({
        status: "success",
        provider: "google",
        mode: null,
      });
    });
  });

  describe("denied variant", () => {
    it("returns denied status with reason", () => {
      const result = readOAuthReturn("?oauth=denied&provider=google&reason=user+cancelled");
      expect(result).toEqual({
        status: "denied",
        provider: "google",
        reason: "user cancelled",
      });
    });

    it("returns denied status with null reason when reason is missing", () => {
      const result = readOAuthReturn("?oauth=denied&provider=google");
      expect(result).toEqual({
        status: "denied",
        provider: "google",
        reason: null,
      });
    });

    it("returns denied status with empty string reason", () => {
      const result = readOAuthReturn("?oauth=denied&provider=google&reason=");
      expect(result).toEqual({
        status: "denied",
        provider: "google",
        reason: "",
      });
    });
  });

  describe("error variant", () => {
    it("returns error status with reason", () => {
      const result = readOAuthReturn("?oauth=error&provider=github&reason=invalid_scope");
      expect(result).toEqual({
        status: "error",
        provider: "github",
        reason: "invalid_scope",
      });
    });

    it("returns error status with null reason when reason is missing", () => {
      const result = readOAuthReturn("?oauth=error&provider=github");
      expect(result).toEqual({
        status: "error",
        provider: "github",
        reason: null,
      });
    });
  });

  describe("missing provider in valid OAuth return", () => {
    it("returns null when provider is missing even if oauth and reason are present", () => {
      expect(readOAuthReturn("?oauth=denied&reason=user+cancelled")).toBeNull();
      expect(readOAuthReturn("?oauth=error&reason=server_error")).toBeNull();
    });
  });

  describe("case sensitivity", () => {
    it("requires exact case match for outcome values", () => {
      expect(readOAuthReturn("?oauth=Success&provider=google")).toBeNull();
      expect(readOAuthReturn("?oauth=DENIED&provider=google")).toBeNull();
      expect(readOAuthReturn("?oauth=Error&provider=google")).toBeNull();
    });

    it("requires exact case match for mode values", () => {
      const result = readOAuthReturn("?oauth=success&provider=google&mode=Session");
      // Session (capital S) is not valid, should be null
      if (result && result.status === "success") {
        expect(result.mode).toBeNull();
      } else {
        throw new Error("Expected success status");
      }
    });
  });

  describe("URL encoding", () => {
    it("correctly decodes URL-encoded parameters", () => {
      const result = readOAuthReturn("?oauth=denied&provider=google&reason=access%20denied");
      expect(result).toEqual({
        status: "denied",
        provider: "google",
        reason: "access denied",
      });
    });

    it("handles special characters in provider name", () => {
      const result = readOAuthReturn("?oauth=success&provider=azure-ad&mode=session");
      expect(result).toEqual({
        status: "success",
        provider: "azure-ad",
        mode: "session",
      });
    });
  });

  describe("invalid oauth outcome", () => {
    it("returns null for unknown outcome values", () => {
      expect(readOAuthReturn("?oauth=unknown&provider=google")).toBeNull();
      expect(readOAuthReturn("?oauth=pending&provider=google")).toBeNull();
      expect(readOAuthReturn("?oauth=&provider=google")).toBeNull();
    });
  });
});
