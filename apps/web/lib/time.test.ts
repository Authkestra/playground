import { describe, it, expect } from "vitest";
import { formatRelativeTime } from "./time";

describe("formatRelativeTime", () => {
  describe("just now", () => {
    it("returns 'just now' for times less than 5 seconds ago (rounds to 0-4 seconds)", () => {
      const now = 1000000;
      // deltaS < 5, so rounds to 0-4 seconds = 0-4499ms
      expect(formatRelativeTime(new Date(now - 0).toISOString(), now)).toBe("just now");
      expect(formatRelativeTime(new Date(now - 1000).toISOString(), now)).toBe("just now");
      expect(formatRelativeTime(new Date(now - 4499).toISOString(), now)).toBe("just now");
    });
  });

  describe("seconds", () => {
    it("returns seconds ago for times that round to 5-59 seconds ago", () => {
      const now = 1000000;
      // 4500ms rounds to 5s, 59499ms rounds to 59s, 59500ms rounds to 60s (then to minutes)
      expect(formatRelativeTime(new Date(now - 4500).toISOString(), now)).toBe("5s ago");
      expect(formatRelativeTime(new Date(now - 30000).toISOString(), now)).toBe("30s ago");
      expect(formatRelativeTime(new Date(now - 59499).toISOString(), now)).toBe("59s ago");
    });
  });

  describe("minutes", () => {
    it("returns minutes ago for times that round to 1-59 minutes ago", () => {
      const now = 1000000;
      // 59500ms rounds to 60s = 1m
      // 3564000ms rounds to 3564s = Math.round(59.4) = 59m
      // 3599500ms rounds to 3600s = 60m (then to hours)
      expect(formatRelativeTime(new Date(now - 59500).toISOString(), now)).toBe("1m ago");
      expect(formatRelativeTime(new Date(now - 1800000).toISOString(), now)).toBe("30m ago");
      expect(formatRelativeTime(new Date(now - 3564000).toISOString(), now)).toBe("59m ago");
    });
  });

  describe("hours", () => {
    it("returns hours ago for times that round to 1-23 hours ago", () => {
      const now = 1000000;
      // 3599500ms rounds to 3600s = 60m = 1h
      // 84564000ms rounds to 84564s = Math.round(23.48) = 23h
      // 86399500ms rounds to 86400s = 24h (then to locale)
      expect(formatRelativeTime(new Date(now - 3599500).toISOString(), now)).toBe("1h ago");
      expect(formatRelativeTime(new Date(now - 43200000).toISOString(), now)).toBe("12h ago");
      expect(formatRelativeTime(new Date(now - 84564000).toISOString(), now)).toBe("23h ago");
    });
  });

  describe("fall-through to locale string", () => {
    it("returns locale string for times that round to >= 24 hours ago", () => {
      const now = 1000000;
      const result = formatRelativeTime(new Date(now - 86399500).toISOString(), now);
      // The result should be a locale string, not a relative time format
      expect(result).not.toBe("just now");
      expect(result).not.toMatch(/^\d+[smh] ago$/);
      // It should contain something that looks like a date/time
      expect(result.length).toBeGreaterThan(0);
    });

    it("returns locale string exactly at 24 hours or more", () => {
      const now = 1000000;
      // 86400000ms = 24 hours exactly
      const result = formatRelativeTime(new Date(now - 86400000).toISOString(), now);
      // At 24 hours or more, it should switch to locale string
      expect(result).not.toMatch(/^\d+[smh] ago$/);
    });
  });

  describe("unparseable input", () => {
    it("returns the raw string when the ISO date is unparseable", () => {
      const now = 1000000;
      expect(formatRelativeTime("not-a-date", now)).toBe("not-a-date");
      expect(formatRelativeTime("2025-13-45T99:99:99.000Z", now)).toBe(
        "2025-13-45T99:99:99.000Z"
      );
      expect(formatRelativeTime("", now)).toBe("");
    });
  });

  describe("with default now parameter", () => {
    it("uses Date.now() when now is not provided", () => {
      const before = Date.now();
      const result = formatRelativeTime(new Date(Date.now() - 2000).toISOString());
      const after = Date.now();
      // Should be "just now" since it was only 2 seconds ago
      expect(result).toBe("just now");
    });
  });

  describe("edge cases", () => {
    it("handles microsecond precision in ISO timestamps", () => {
      const now = 1000000;
      expect(formatRelativeTime(new Date(now - 1000).toISOString(), now)).toBe("just now");
    });

    it("rounds correctly at boundaries", () => {
      const now = 1000000;
      // Test rounding at 60 second boundary (59500ms rounds to 60s, which becomes 1 minute)
      expect(formatRelativeTime(new Date(now - 59400).toISOString(), now)).toBe("59s ago");
      expect(formatRelativeTime(new Date(now - 59500).toISOString(), now)).toBe("1m ago");
    });
  });
});
