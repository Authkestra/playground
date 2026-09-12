"use client";

import { useState } from "react";
import type { Forgery, IssuedToken, ProtectedCall } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";
import { cn } from "@/lib/cn";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-flow. */
  onDemoDisabled: () => void;
}

/**
 * Colour by what the outcome means, not by HTTP status.
 *
 * A 401 is the normal answer here, so destructive is reserved for the two
 * outcomes that mean something is actually wrong: a signature that does not
 * verify (someone forged or tampered), and a key set that could not be
 * fetched (the resource server is broken, not under attack).
 */
export function verdictStyle(verdict: string): string {
  switch (verdict) {
    case "accepted":
      return "border-success/30 bg-success/10 text-success-foreground";
    case "absent":
      return "border-border bg-muted/60 text-muted-foreground";
    case "bad_signature":
    case "keys_unreachable":
      return "border-destructive/30 bg-destructive/10 text-destructive-foreground";
    default:
      return "border-warning/30 bg-warning/10 text-warning-foreground";
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
      <p className="text-sm text-muted-foreground">
        A route that validates a token it did not issue. It holds no secret — only the
        issuer&apos;s name and the URL of its published keys. Issue a token, then present
        it, then present one built to fail and watch the reason change.
      </p>

      {banner && (
        <Alert className="border-warning/30 bg-warning/10 text-warning-foreground [&>svg]:text-warning-foreground">
          <AlertDescription>{banner}</AlertDescription>
        </Alert>
      )}

      <div className="flex flex-wrap gap-2">
        <Button type="button" size="sm" onClick={() => void mint("issue")} disabled={working}>
          {busy === "issue" ? "Issuing…" : "Issue a valid token"}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => void call(token || null)}
          disabled={working}
        >
          {busy === "call" ? "Calling…" : "Call the route"}
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={() => void call(null)} disabled={working}>
          Call with no token
        </Button>
      </div>

      <div>
        <p className="text-xs text-muted-foreground">
          Or mint a token built to fail one specific check. Each is really signed — the
          verdict comes from real validation, not from a label.
        </p>
        <div className="mt-2 flex flex-wrap gap-2">
          {FORGERIES.map((f) => (
            <Button
              key={f.kind}
              type="button"
              size="sm"
              variant="outline"
              onClick={() => void mint("forge", f.kind)}
              disabled={working}
            >
              {busy === f.kind ? "Minting…" : f.label}
            </Button>
          ))}
        </div>
      </div>

      {issued && (
        <Card>
          <CardContent className="p-3">
            {issued.forged_as ? (
              <p className="text-xs text-warning-foreground">
                This one is built to fail: it {issued.forged_as}.
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">A valid token, signed by the live key.</p>
            )}

            <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
              <dt className="text-muted-foreground">kid</dt>
              <dd className="break-all font-mono text-foreground">{issued.kid ?? "(none)"}</dd>
              <dt className="text-muted-foreground">iss</dt>
              <dd className="break-all font-mono text-foreground">{issued.issuer}</dd>
              <dt className="text-muted-foreground">aud</dt>
              <dd className="break-all font-mono text-foreground">{issued.audience}</dd>
            </dl>

            {decoded && (
              <div className="mt-2">
                <p className="text-xs text-muted-foreground">Decoded header</p>
                <pre className="mt-1 overflow-x-auto rounded-md bg-muted p-2 font-mono text-xs text-muted-foreground">
                  <code>{decoded}</code>
                </pre>
              </div>
            )}

            {/*
              The invitation that makes "unpublished key" checkable rather than
              asserted. A visitor who opens this and searches for the kid above
              has verified the demo instead of believing it.
            */}
            <p className="mt-2 text-xs text-muted-foreground">
              Look the <code className="font-mono">kid</code> up yourself:{" "}
              <a
                href={issued.jwks_url}
                target="_blank"
                rel="noreferrer"
                className="font-medium text-foreground underline underline-offset-2"
              >
                {issued.jwks_url}
              </a>
            </p>

            <Label className="mt-3 block text-xs font-normal text-muted-foreground" htmlFor="resource-token">
              Edit before calling to see the rest — change a character in the last segment
              for a bad signature, or delete one to make it unreadable. Whatever is here is
              what gets sent.
            </Label>
            <textarea
              id="resource-token"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              spellCheck={false}
              rows={3}
              className="mt-1 w-full resize-y break-all rounded-md border border-input bg-transparent p-2 font-mono text-xs text-foreground placeholder:text-muted-foreground"
            />
          </CardContent>
        </Card>
      )}

      <div aria-live="polite">
        {sentHeader && (
          <p className="mb-1 break-all font-mono text-[11px] text-muted-foreground">
            GET /api/protected · {sentHeader}
          </p>
        )}
        {result && (
          <div className={cn("rounded-md border px-3 py-2 text-sm", verdictStyle(result.verdict))}>
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
