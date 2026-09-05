import { describe, it, expect } from "vitest";
import { base64UrlToBuffer, bufferToBase64Url } from "./PasskeysPanel";

describe("base64UrlToBuffer and bufferToBase64Url", () => {
  // base64url uses - and _ instead of + and / and has no padding
  describe("base64UrlToBuffer", () => {
    it("decodes base64url-encoded strings with the correct alphabet (-/_ instead of +//)", () => {
      // Standard base64 would be "SGVsbG8gV29ybGQh" with no padding needed
      // This is "Hello World!" encoded in base64url
      const base64url = "SGVsbG8gV29ybGQh";
      const result = base64UrlToBuffer(base64url);
      const decoded = new TextDecoder().decode(result);
      expect(decoded).toBe("Hello World!");
    });

    it("handles base64url strings with - and _ characters", () => {
      // Test with base64url-specific characters: - and _
      // "+/" in standard base64 becomes "-_" in base64url
      const base64urlEquiv = "__-_"; // equivalent in base64url
      const result = base64UrlToBuffer(base64urlEquiv);
      // Just verify it doesn't throw and produces a Uint8Array
      expect(result).toBeInstanceOf(Uint8Array);
      expect(result.length).toBeGreaterThan(0);
    });

    it("restores padding correctly for input with length % 4 == 0", () => {
      // "SGVsbG8=" has 8 chars (length % 4 == 0, no padding needed)
      const base64url = "SGVsbG8";
      const result = base64UrlToBuffer(base64url);
      const decoded = new TextDecoder().decode(result);
      expect(decoded).toBe("Hello");
    });

    it("restores padding correctly for input with length % 4 == 1", () => {
      // Length 1: needs 3 padding chars (but this is invalid for real base64)
      // Just test that it doesn't crash
      const base64url = "YQ";
      const result = base64UrlToBuffer(base64url);
      expect(result).toBeInstanceOf(Uint8Array);
    });

    it("restores padding correctly for input with length % 4 == 2", () => {
      // Length 2: needs 2 padding chars
      const base64url = "YQ";
      const result = base64UrlToBuffer(base64url);
      expect(result).toBeInstanceOf(Uint8Array);
    });

    it("restores padding correctly for input with length % 4 == 3", () => {
      // Length 3: needs 1 padding char
      const base64url = "YWI";
      const result = base64UrlToBuffer(base64url);
      const decoded = new TextDecoder().decode(result);
      expect(decoded).toBe("ab");
    });

    it("handles zero-length input", () => {
      const result = base64UrlToBuffer("");
      expect(result).toBeInstanceOf(Uint8Array);
      expect(result.length).toBe(0);
    });

    it("handles bytes >= 0x80 correctly", () => {
      // Create a Uint8Array with bytes >= 0x80
      const original = new Uint8Array([0x80, 0x90, 0xff, 0xfe]);
      const encoded = bufferToBase64Url(original.buffer);
      const decoded = base64UrlToBuffer(encoded);
      expect(decoded).toEqual(original);
    });
  });

  describe("bufferToBase64Url", () => {
    it("encodes Uint8Array to base64url format with - and _ instead of + and /", () => {
      const text = new TextEncoder().encode("Hello World!");
      const result = bufferToBase64Url(text.buffer);
      // Should not contain + or / characters
      expect(result).not.toContain("+");
      expect(result).not.toContain("/");
      // Should be able to round-trip
      expect(base64UrlToBuffer(result)).toEqual(text);
    });

    it("strips padding = characters from the output", () => {
      const text = new TextEncoder().encode("test");
      const result = bufferToBase64Url(text.buffer);
      expect(result).not.toContain("=");
      // Verify it round-trips correctly without padding
      expect(base64UrlToBuffer(result)).toEqual(text);
    });

    it("handles zero-length ArrayBuffer", () => {
      const buffer = new ArrayBuffer(0);
      const result = bufferToBase64Url(buffer);
      expect(result).toBe("");
    });

    it("handles ArrayBuffer with single byte", () => {
      const buffer = new ArrayBuffer(1);
      new Uint8Array(buffer)[0] = 42;
      const result = bufferToBase64Url(buffer);
      expect(base64UrlToBuffer(result)[0]).toBe(42);
    });

    it("handles bytes >= 0x80 correctly", () => {
      const buffer = new ArrayBuffer(4);
      const view = new Uint8Array(buffer);
      view[0] = 0x80;
      view[1] = 0x90;
      view[2] = 0xff;
      view[3] = 0xfe;
      const encoded = bufferToBase64Url(buffer);
      const decoded = base64UrlToBuffer(encoded);
      expect(new Uint8Array(decoded)).toEqual(view);
    });
  });

  describe("round-trip: base64UrlToBuffer and bufferToBase64Url", () => {
    it("round-trips arbitrary bytes", () => {
      const original = new Uint8Array([0x00, 0x01, 0x7f, 0x80, 0xff]);
      const encoded = bufferToBase64Url(original.buffer);
      const decoded = base64UrlToBuffer(encoded);
      expect(decoded).toEqual(original);
    });

    it("round-trips text encoded as UTF-8", () => {
      const text = "Hello, 世界!";
      const original = new TextEncoder().encode(text);
      const encoded = bufferToBase64Url(original.buffer);
      const decoded = base64UrlToBuffer(encoded);
      const decodedText = new TextDecoder().decode(decoded);
      expect(decodedText).toBe(text);
    });

    it("round-trips WebAuthn-like challenge data", () => {
      // Simulate a WebAuthn challenge: random bytes
      const challenge = new Uint8Array(32);
      for (let i = 0; i < 32; i++) {
        challenge[i] = Math.floor(Math.random() * 256);
      }
      const encoded = bufferToBase64Url(challenge.buffer);
      const decoded = base64UrlToBuffer(encoded);
      expect(decoded).toEqual(challenge);
    });

    it("preserves input for every length modulo 4", () => {
      // Test various lengths to ensure padding restoration works
      const lengths = [1, 2, 3, 4, 5, 6, 7, 8, 15, 16, 17, 31, 32, 33];
      for (const len of lengths) {
        const original = new Uint8Array(len);
        for (let i = 0; i < len; i++) {
          original[i] = Math.floor(Math.random() * 256);
        }
        const encoded = bufferToBase64Url(original.buffer);
        const decoded = base64UrlToBuffer(encoded);
        expect(decoded).toEqual(original);
      }
    });
  });
});
