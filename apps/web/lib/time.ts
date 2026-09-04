/**
 * Formats an RFC3339 timestamp as a short relative time ("just now", "5s
 * ago", "12m ago", ...). Falls back to a locale time string once it's more
 * than a day old, and to the raw string if it doesn't parse.
 */
export function formatRelativeTime(iso: string, now: number = Date.now()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;

  const deltaMs = now - then;
  const deltaS = Math.round(deltaMs / 1000);

  if (deltaS < 5) return "just now";
  if (deltaS < 60) return `${deltaS}s ago`;
  const deltaM = Math.round(deltaS / 60);
  if (deltaM < 60) return `${deltaM}m ago`;
  const deltaH = Math.round(deltaM / 60);
  if (deltaH < 24) return `${deltaH}h ago`;

  return new Date(iso).toLocaleString();
}
