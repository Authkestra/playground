"use client";

import { useState } from "react";
import type { Forgery, IssuedToken, ProtectedCall } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-flow. */
  onDemoDisabled: () => void;
}

/**
 * Colour by what the outcome means, not by HTTP status.
 *
 * A 401 is the normal answer here, so red is reserved for the two outcomes
 * that mean something is actually wrong: a signature that does not verify
 * (someone forged or tampered), and a key set that could not be fetched (the
 * resource server is broken, not under attack).
 */
export function verdictStyle(verdict: string): string {
  switch (verdict) {
    case "accepted":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-300";
    case "absent":
      return "border-slate-600 bg-slate-800/60 text-slate-300";
    case "bad_signature":
    case "keys_unreachable":
      return "border-rose-500/30 bg-rose-500/10 text-rose-300";
    default:
      return "border-amber-500/30 bg-amber-500/10 text-amber-300";
  }
}

export const VERDICT_LABELS: Record<string, string> = {
  accepted: "Accepted",
  absent: "No token sent",
  malformed: "Malformed token",
  missing_kid: "No `kid` header",
  unknown_kid: "Unpublished key",
  untrusted_issuer: "Untrusted issuer",
  wrong_issuer: "Wrong issuer",
  wrong_audience: "Wrong audience",
  expired: "Expired token",
  bad_signature: "Bad signature",
  keys_unreachable: "Key set unreachable",
  rejected: "Refused",
};

/** Every forgery the API can mint, and what each one is for. */
export const FORGERIES: { kind: Forgery; label: string }[] = [
  { kind: "unknown_kid", label: "Unpublished key" },
  { kind: "bad_signature", label: "Real `kid`, wrong key" },
  { kind: "untrusted_issuer", label: "Untrusted issuer" },
  { kind: "wrong_audience", label: "Another service" },
  { kind: "expired", label: "Expired" },
  { kind: "missing_kid", label: "No `kid`" },
];

/** The header segment of a JWT, decoded. Shown so the `kid` is not hearsay. */
export function decodeHeader(token: string): string | null {
  const segment = token.split(".")[0];
  if (!segment) return null;
  try {
    const json = atob(segment.replace(/-/g, "+").replace(/_/g, "/"));
    return JSON.stringify(JSON.parse(json));
  } catch {
    return null;
  }
}

export default function ResourcePanel({ scenarioId, onDemoDisabled }: Props) {
  const [issued, setIssued] = useState<IssuedToken | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<ProtectedCall | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [token, setToken] = useState("");
  // The request the current result came from. Without it the panel only
  // asserts an outcome; "called with no token, got a 401" is a claim until you
  // can see the header that was, or was not, sent.
  const [sentHeader, setSentHeader] = useState<string | null>(null);

  async function mint(action: "issue" | "forge", kind?: Forgery) {
    setBusy(kind ?? action);
    setBanner(null);
    const res = await scenarioAction<IssuedToken>(scenarioId, action, kind ? { kind } : {});
    setBusy(null);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") return onDemoDisabled();
      setBanner(
        res.error.kind === "rate_limited" || res.error.kind === "http_error"
          ? res.error.detail
          : "Could not reach the API to mint a token.",
      );
      return;
    }
    setIssued(res.data);
    setToken(res.data.token);
    setResult(null);
    setSentHeader(null);
  }

  function headerFor(value: string | null): string {
    const trimmed = value?.trim();
    return trimmed
      ? `Authorization: Bearer ${trimmed.length > 48 ? `${trimmed.slice(0, 48)}…` : trimmed}`
      : "(no Authorization header)";
  }

  async function call(value: string | null) {
    setBusy("call");
    setBanner(null);
    setSentHeader(headerFor(value));
    const res = await scenarioAction<ProtectedCall>(scenarioId, "call", {
      token: value ?? undefined,
    });
    setBusy(null);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") return onDemoDisabled();
      setBanner(
        res.error.kind === "rate_limited" || res.error.kind === "http_error"
          ? res.error.detail
          : "Could not reach the API.",
      );
      return;
    }
    setResult(res.data);
  }

  const working = busy !== null;
  const decoded = issued ? decodeHeader(token) : null;

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-slate-400">
        A route that validates a token it did not issue. It holds no secret — only the
        issuer&apos;s name and the URL of its published keys. Issue a token, then present
        it, then present one built to fail and watch the reason change.
      </p>

      {banner && (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-300">
          {banner}
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void mint("issue")}
          disabled={working}
          className="rounded-md bg-emerald-500 px-3 py-1.5 text-sm font-semibold text-slate-950 transition hover:bg-emerald-400 disabled:cursor-not-allowed disabled:bg-emerald-500/50"
        >
          {busy === "issue" ? "Issuing…" : "Issue a valid token"}
        </button>
        <button
          type="button"
          onClick={() => void call(token || null)}
          disabled={working}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy === "call" ? "Calling…" : "Call the route"}
        </button>
        <button
          type="button"
          onClick={() => void call(null)}
          disabled={working}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Call with no token
        </button>
      </div>

      <div>
        <p className="text-xs text-slate-400">
          Or mint a token built to fail one specific check. Each is really signed — the
          verdict comes from real validation, not from a label.
        </p>
        <div className="mt-2 flex flex-wrap gap-2">
          {FORGERIES.map((f) => (
            <button
              key={f.kind}
              type="button"
              onClick={() => void mint("forge", f.kind)}
              disabled={working}
              className="rounded-md border border-slate-700 px-2.5 py-1 text-xs font-medium text-slate-300 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {busy === f.kind ? "Minting…" : f.label}
            </button>
          ))}
        </div>
      </div>

      {issued && (
        <div className="rounded-md border border-slate-800 bg-slate-900/60 p-3">
          {issued.forged_as ? (
            <p className="text-xs text-amber-400">
              This one is built to fail: it {issued.forged_as}.
            </p>
          ) : (
            <p className="text-xs text-slate-400">A valid token, signed by the live key.</p>
          )}

          <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            <dt className="text-slate-400">kid</dt>
            <dd className="break-all font-mono text-slate-300">{issued.kid ?? "(none)"}</dd>
            <dt className="text-slate-400">iss</dt>
            <dd className="break-all font-mono text-slate-300">{issued.issuer}</dd>
            <dt className="text-slate-400">aud</dt>
            <dd className="break-all font-mono text-slate-300">{issued.audience}</dd>
            {decoded && (
              <>
                <dt className="text-slate-400">header</dt>
                <dd className="break-all font-mono text-slate-300">{decoded}</dd>
              </>
            )}
          </dl>

          {/*
            The invitation that makes "unpublished key" checkable rather than
            asserted. A visitor who opens this and searches for the kid above
            has verified the demo instead of believing it.
          */}
          <p className="mt-2 text-xs text-slate-400">
            Look the <code className="font-mono">kid</code> up yourself:{" "}
            <a
              href={issued.jwks_url}
              target="_blank"
              rel="noreferrer"
              className="font-medium text-slate-200 underline underline-offset-2"
            >
              {issued.jwks_url}
            </a>
          </p>

          <label className="mt-3 block text-xs text-slate-400" htmlFor="resource-token">
            Edit before calling to see the rest — change a character in the last segment
            for a bad signature, or delete one to make it unreadable. Whatever is here is
            what gets sent.
          </label>
          <textarea
            id="resource-token"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            spellCheck={false}
            rows={3}
            className="mt-1 w-full resize-y break-all rounded border border-slate-700 bg-slate-950 p-2 font-mono text-xs text-slate-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-400"
          />
        </div>
      )}

      <div aria-live="polite">
        {sentHeader && (
          <p className="mb-1 break-all font-mono text-[11px] text-slate-400">
            GET /api/protected · {sentHeader}
          </p>
        )}
        {result && (
          <div className={`rounded-md border px-3 py-2 text-sm ${verdictStyle(result.verdict)}`}>
            <p className="font-medium">
              {result.status} · {VERDICT_LABELS[result.verdict] ?? result.verdict}
            </p>
            <p className="mt-1 whitespace-pre-line text-xs opacity-90">{result.detail}</p>
            {result.subject && (
              <p className="mt-1 font-mono text-xs opacity-75">sub: {result.subject}</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
