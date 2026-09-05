"use client";

import { useEffect, useState } from "react";
import type { DemoSessionView } from "@playground/api-types";

interface Props {
  session: DemoSessionView | null;
  onReset: () => void;
  resetting: boolean;
}

interface ExpiryDisplay {
  relative: string;
  absolute: string;
}

/**
 * Formats a future-dated RFC3339 timestamp as a countdown ("Expires in 11h",
 * "Expires in 45m", "Expires in 30s", or "Expired" if already past). Returns
 * the relative countdown and absolute locale time string for use as a title.
 * Falls back to null if the timestamp doesn't parse.
 */
function formatExpiry(iso: string, now: number = Date.now()): ExpiryDisplay | null {
  const expiryTime = new Date(iso).getTime();
  if (Number.isNaN(expiryTime)) return null;

  const absolute = new Date(iso).toLocaleString();
  const deltaMs = expiryTime - now;
  const deltaS = Math.floor(deltaMs / 1000);

  if (deltaS < 0) {
    return { relative: "Expired", absolute };
  }

  const deltaM = Math.floor(deltaS / 60);
  const deltaH = Math.floor(deltaM / 60);

  let relative: string;
  if (deltaH > 0) {
    relative = `Expires in ${deltaH}h`;
  } else if (deltaM > 0) {
    relative = `Expires in ${deltaM}m`;
  } else {
    relative = `Expires in ${deltaS}s`;
  }

  return { relative, absolute };
}

export default function SessionBar({ session, onReset, resetting }: Props) {
  // Initialized to null to avoid server/client render mismatch: the countdown
  // depends on Date.now(), which will differ between server and browser on
  // first render. We compute it in an effect so both renders match, then
  // update with the real countdown.
  const [expiryDisplay, setExpiryDisplay] = useState<ExpiryDisplay | null>(null);

  useEffect(() => {
    if (!session) {
      setExpiryDisplay(null);
      return;
    }

    // Compute initial countdown to match browser's current time.
    const result = formatExpiry(session.expires_at);
    setExpiryDisplay(result);

    // Update every minute to keep the countdown honest. Since the session
    // expiry is far enough away that seconds don't matter, one minute is plenty.
    const interval = setInterval(() => {
      const updated = formatExpiry(session.expires_at);
      setExpiryDisplay(updated);
    }, 60_000);

    return () => clearInterval(interval);
  }, [session]);

  return (
    <div className="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900 px-4 py-3 text-sm">
      <div className="flex flex-col gap-0.5">
        <span className="font-medium text-slate-200">Demo session</span>
        {session ? (
          <span className="text-slate-400" title={expiryDisplay?.absolute}>
            {/*
              Before the effect has run there is no countdown yet. Fall back to
              the absolute form rather than the raw RFC3339 string: this renders
              for one frame after the session loads, and a flash of
              `2026-09-06T09:13:44.123456+00:00` is worse than the timestamp
              this component showed before it learned to count down.
            */}
            {expiryDisplay
              ? expiryDisplay.relative
              : `Expires ${new Date(session.expires_at).toLocaleString()}`}
          </span>
        ) : (
          <span className="text-slate-500">No session yet</span>
        )}
      </div>
      <button
        type="button"
        onClick={onReset}
        disabled={resetting}
        className="rounded-md border border-slate-700 px-3 py-1.5 font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {resetting ? "Resetting…" : "Reset session"}
      </button>
    </div>
  );
}
