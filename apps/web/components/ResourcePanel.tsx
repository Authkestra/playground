"use client";

import { useState } from "react";
import type { IssuedToken, ProtectedCall } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-flow. */
  onDemoDisabled: () => void;
  /** Called after every round trip, so the flow log can refetch. */
  onAction?: () => void;
}

/** Colour by outcome, not by HTTP status — a 401 is the normal answer here. */
const VERDICT_STYLES: Record<string, string> = {
  accepted: "border-emerald-500/30 bg-emerald-500/10 text-emerald-300",
  absent: "border-slate-600 bg-slate-800/60 text-slate-300",
  malformed: "border-amber-500/30 bg-amber-500/10 text-amber-300",
  expired: "border-amber-500/30 bg-amber-500/10 text-amber-300",
  wrong_audience: "border-amber-500/30 bg-amber-500/10 text-amber-300",
  bad_signature: "border-rose-500/30 bg-rose-500/10 text-rose-300",
};

const VERDICT_LABELS: Record<string, string> = {
  accepted: "Accepted",
  absent: "No token sent",
  malformed: "Malformed token",
  expired: "Expired token",
  wrong_audience: "Wrong audience",
  bad_signature: "Bad signature",
};

export default function ResourcePanel({ scenarioId, onDemoDisabled, onAction }: Props) {
  const [issued, setIssued] = useState<IssuedToken | null>(null);
  const [issuing, setIssuing] = useState(false);
  const [calling, setCalling] = useState(false);
  const [result, setResult] = useState<ProtectedCall | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [pasted, setPasted] = useState("");

  async function issue() {
    setIssuing(true);
    setBanner(null);
    const res = await scenarioAction<IssuedToken>(scenarioId, "issue");
    setIssuing(false);
    onAction?.();

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") return onDemoDisabled();
      setBanner(res.error.kind === "rate_limited" ? res.error.detail : "Could not issue a token.");
      return;
    }
    setIssued(res.data);
    setPasted(res.data.token);
    setResult(null);
  }

  async function call(token: string | null) {
    setCalling(true);
    setBanner(null);
    const res = await scenarioAction<ProtectedCall>(scenarioId, "call", {
      token: token ?? undefined,
    });
    setCalling(false);
    onAction?.();

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") return onDemoDisabled();
      setBanner(res.error.kind === "rate_limited" ? res.error.detail : "The call failed.");
      return;
    }
    setResult(res.data);
  }

  const busy = issuing || calling;

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-slate-400">
        A route that serves nobody without a valid token. Issue one, call it, then call
        it again with the token removed or altered and watch the answer change.
      </p>

      {banner && (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-300">
          {banner}
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void issue()}
          disabled={busy}
          className="rounded-md bg-emerald-500 px-3 py-1.5 text-sm font-semibold text-slate-950 transition hover:bg-emerald-400 disabled:cursor-not-allowed disabled:bg-emerald-500/50"
        >
          {issuing ? "Issuing…" : "Issue a token"}
        </button>

        <button
          type="button"
          onClick={() => void call(pasted || null)}
          disabled={busy}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {calling ? "Calling…" : "Call with this token"}
        </button>

        <button
          type="button"
          onClick={() => void call(null)}
          disabled={busy}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Call with no token
        </button>
      </div>

      {issued && (
        <div className="rounded-md border border-slate-800 bg-slate-900/60 p-3">
          <p className="text-xs text-slate-400">
            Audience <code className="font-mono text-slate-400">{issued.audience}</code> ·
            expires in {issued.expires_in}s
          </p>
          <label className="mt-2 block text-xs text-slate-400" htmlFor="token">
            Edit it before calling to see the other failures — change a character for a bad
            signature, or delete a segment for a malformed one.
          </label>
          <textarea
            id="token"
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
            spellCheck={false}
            rows={3}
            className="mt-1 w-full resize-y break-all rounded border border-slate-700 bg-slate-950 p-2 font-mono text-xs text-slate-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-400"
          />
        </div>
      )}

      <div aria-live="polite">
        {result && (
          <div
            className={`rounded-md border px-3 py-2 text-sm ${
              VERDICT_STYLES[result.verdict] ?? VERDICT_STYLES.absent
            }`}
          >
            <p className="font-medium">
              {result.status} · {VERDICT_LABELS[result.verdict] ?? result.verdict}
            </p>
            <p className="mt-1 text-xs opacity-90">{result.detail}</p>
            {result.subject && (
              <p className="mt-1 font-mono text-xs opacity-75">sub: {result.subject}</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
