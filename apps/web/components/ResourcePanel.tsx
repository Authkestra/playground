"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { Forgery, IssuedToken, ProtectedCall } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";
import { cn } from "@/lib/cn";
import {
  checkEd25519Support,
  decodeClaims,
  decodeHeader,
  describeExpiry,
  describeVerdict,
  fetchJwks,
  formatAudience,
  verifyTokenSignature,
  type Ed25519Support,
  type Jwk,
  type LocalVerdict,
} from "@/lib/jwt";
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

/**
 * What to call the verdict the *browser* reached, in the same words the API's
 * verdict uses where the two are answering the same question.
 *
 * Shared vocabulary is the point: a visitor comparing "Unpublished key" here
 * against "Unpublished key" from the API is comparing two independent answers,
 * and a disagreement between them would be visible rather than buried in
 * different phrasing.
 */
export const LOCAL_VERDICT_LABELS: Record<LocalVerdict["kind"], string> = {
  verified: "Signature verified here",
  bad_signature: "Bad signature",
  unknown_kid: "Unpublished key",
  missing_kid: "No `kid` header",
  malformed: "Malformed token",
  unsupported_alg: "Unchecked algorithm",
  unsupported: "Cannot be checked here",
};

/**
 * Colour by what the local verdict means, on the same rule as
 * [`verdictStyle`]: destructive is reserved for a signature that does not
 * verify, because a rejection is the ordinary answer in this scenario and a UI
 * that shouts at every 401 teaches people to ignore it.
 *
 * "Cannot be checked here" is deliberately neutral rather than alarming. A
 * browser without Ed25519 is not a finding about the token.
 */
export function localVerdictStyle(kind: LocalVerdict["kind"]): string {
  switch (kind) {
    case "verified":
      return "border-success/30 bg-success/10 text-success-foreground";
    case "bad_signature":
      return "border-destructive/30 bg-destructive/10 text-destructive-foreground";
    case "unsupported":
    case "unsupported_alg":
      return "border-border bg-muted/60 text-muted-foreground";
    default:
      return "border-warning/30 bg-warning/10 text-warning-foreground";
  }
}

/**
 * The same two functions the buttons call, put somewhere a visitor can reach
 * them without our UI in the way.
 *
 * A panel that says "this ran in your browser" is still the panel saying it.
 * `authkestra.verify(token)` in the console is the same code with our
 * rendering removed — a visitor can paste a token of their own, or one of ours
 * with a character changed, and watch the answer change for a reason they
 * chose.
 */
export interface ConsoleHandle {
  /** Verifies against the key set already in this tab. Makes no request. */
  verify(token: string, keys?: Jwk[]): Promise<LocalVerdict>;
  /** The one call that does touch the network, kept separate on purpose. */
  fetchKeys(url?: string): Promise<Jwk[]>;
  /** Whatever the "Fetch the key set" button last brought back. */
  keys: Jwk[] | null;
  /** Where those keys came from. */
  jwksUrl: string | null;
}

declare global {
  interface Window {
    authkestra?: ConsoleHandle;
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

  // The visitor's own half of the scenario. `keys` is what they fetched, and
  // it is kept across tokens on purpose: fetch once, then verify as many
  // tokens as you like with the network switched off.
  const [keys, setKeys] = useState<Jwk[] | null>(null);
  const [keysAt, setKeysAt] = useState<string | null>(null);
  const [keysError, setKeysError] = useState<string | null>(null);
  const [local, setLocal] = useState<{ verdict: LocalVerdict; at: string } | null>(null);
  const [support, setSupport] = useState<Ed25519Support | null>(null);

  // Read by the console handle, which is installed once and must not go stale.
  const keysRef = useRef<Jwk[] | null>(null);
  const jwksUrlRef = useRef<string | null>(null);
  useEffect(() => {
    keysRef.current = keys;
  }, [keys]);
  useEffect(() => {
    jwksUrlRef.current = issued?.jwks_url ?? null;
  }, [issued]);

  // `exp` is judged against the visitor's clock, so the clock has to actually
  // run: a token with a sixty-second life should be seen to expire, not be
  // reported as expired only because something else caused a re-render.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!issued) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [issued]);

  // Asked once, up front, so a browser that cannot do Ed25519 is told before
  // it presses the button rather than after.
  useEffect(() => {
    let cancelled = false;
    void checkEd25519Support().then((result) => {
      if (!cancelled) setSupport(result);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // The console handle. Installed on mount and removed on unmount, because a
  // global that outlives the panel that explains it is just litter.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const handle: ConsoleHandle = {
      verify: (value, override) => verifyTokenSignature(value, override ?? keysRef.current ?? []),
      fetchKeys: (url) => {
        const target = url ?? jwksUrlRef.current;
        if (!target) return Promise.reject(new Error("no key set URL yet — issue a token first"));
        return fetchJwks(target);
      },
      get keys() {
        return keysRef.current;
      },
      get jwksUrl() {
        return jwksUrlRef.current;
      },
    };
    window.authkestra = handle;
    return () => {
      if (window.authkestra === handle) delete window.authkestra;
    };
  }, []);

  const loadKeys = useCallback(async (url: string) => {
    setBusy("jwks");
    setKeysError(null);
    // The verdict names the key set it was computed against, so it cannot
    // outlive a fetch of a different one.
    setLocal(null);
    try {
      const fetched = await fetchJwks(url);
      setKeys(fetched);
      setKeysAt(new Date().toLocaleTimeString());
    } catch (error) {
      setKeys(null);
      setKeysAt(null);
      setKeysError(error instanceof Error ? error.message : "the key set could not be fetched");
    } finally {
      setBusy(null);
    }
  }, []);

  /**
   * The verdict the visitor can check. Deliberately `await`s nothing but
   * WebCrypto: everything it needs is already in the tab.
   */
  const verifyHere = useCallback(async (value: string, fetched: Jwk[]) => {
    setBusy("verify");
    const verdict = await verifyTokenSignature(value, fetched);
    setLocal({ verdict, at: new Date().toLocaleTimeString() });
    setBusy(null);
  }, []);

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
    // A verdict about the previous token would be actively misleading beside
    // this one. The fetched key set stays, because it is about the issuer.
    setLocal(null);
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
  // Decoded from whatever is in the textarea, on every render, so editing the
  // token changes what is shown before anything is sent anywhere.
  const header = issued ? decodeHeader(token) : null;
  const claims = issued ? decodeClaims(token) : null;
  const expiry = claims ? describeExpiry(claims.exp, now) : null;
  const tokenAudience = claims ? formatAudience(claims.aud) : null;

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

            {/*
              The route's policy, which is the API's word — and the only thing
              here that has to be. Everything below is read out of the token,
              so the two can be compared rather than conflated.
            */}
            <p className="mt-2 text-xs text-muted-foreground">
              This route accepts <code className="font-mono text-foreground">{issued.audience}</code>{" "}
              in <code className="font-mono">aud</code>, from{" "}
              <code className="font-mono text-foreground">{issued.issuer}</code>.
            </p>

            {/*
              Tier 1 of #76. `decodeHeader` already showed the `kid` so it was
              not hearsay; the claims are where the *reason* for a rejection
              lives, and four of the six forgeries explain themselves the
              moment they are visible.
            */}
            <p className="mt-3 text-xs text-muted-foreground">
              Decoded from the token in this tab. Nothing below was sent to us, or by us:
            </p>
            <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
              <dt className="text-muted-foreground">kid</dt>
              <dd className="break-all font-mono text-foreground">
                {typeof header?.kid === "string" ? (
                  header.kid
                ) : (
                  <span className="text-warning-foreground">
                    (absent — nothing says which key to check)
                  </span>
                )}
              </dd>

              <dt className="text-muted-foreground">iss</dt>
              <dd className="break-all font-mono text-foreground">
                {claims?.iss ?? "(absent)"}
                {claims?.iss && claims.iss !== issued.issuer && (
                  <span className="ml-1 font-sans text-warning-foreground">
                    — not the issuer this route trusts
                  </span>
                )}
              </dd>

              <dt className="text-muted-foreground">aud</dt>
              <dd className="break-all font-mono text-foreground">
                {tokenAudience ?? "(absent)"}
                {tokenAudience && tokenAudience !== issued.audience && (
                  <span className="ml-1 font-sans text-warning-foreground">
                    — another service, not this one
                  </span>
                )}
              </dd>

              <dt className="text-muted-foreground">exp</dt>
              <dd
                className={cn(
                  "break-all font-mono",
                  expiry?.expired ? "text-warning-foreground" : "text-foreground",
                )}
              >
                {expiry ? expiry.text : "(absent)"}
              </dd>

              <dt className="text-muted-foreground">sub</dt>
              <dd className="break-all font-mono text-foreground">{claims?.sub ?? "(absent)"}</dd>
            </dl>

            {header && (
              <div className="mt-2">
                <p className="text-xs text-muted-foreground">Raw header</p>
                <pre className="mt-1 overflow-x-auto rounded-md bg-muted p-2 font-mono text-xs text-muted-foreground">
                  <code>{JSON.stringify(header)}</code>
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
              onChange={(e) => {
                setToken(e.target.value);
                // A verdict about bytes that are no longer in the box is worse
                // than no verdict: change one character and the badge below
                // goes away until you ask again.
                setLocal(null);
              }}
              spellCheck={false}
              rows={3}
              className="mt-1 w-full resize-y break-all rounded-md border border-input bg-transparent p-2 font-mono text-xs text-foreground placeholder:text-muted-foreground"
            />
          </CardContent>
        </Card>
      )}

      {/*
        Tier 2 of #76: the same question the API answers, answered again by
        the one party in this scenario that we do not control.

        The two steps are separate on purpose, and the separation is the
        evidence. Fetching the key set is a visible request the visitor makes;
        verifying is silent. If verification fetched its own keys there would
        be no moment at which this page can be seen answering without us.
      */}
      {issued && (
        <Card>
          <CardContent className="p-3">
            <p className="text-xs text-muted-foreground">
              Everything above is our word for it. Below, your browser answers the same
              question — first fetch the published keys, then verify against them here.
            </p>
            {/*
              Said before the verdict, because without it the two answers look
              like they contradict each other. A wrong-audience token really
              does verify: it was signed by the live key. The API refuses it on
              policy, which is a different question from whether the bytes are
              authentic — and the claims above are where that question is
              answered, by the visitor, for themselves.
            */}
            <p className="mt-2 text-xs text-muted-foreground">
              Your browser answers only the cryptographic half — did a published key sign
              these exact bytes. Whether the issuer is trusted, the audience is this service
              and the clock has run out is policy, which you read off the claims above. An
              expired or wrongly-addressed token verifies here and is still refused there;
              that is agreement, not conflict.
            </p>

            {support?.supported === false && (
              <Alert className="mt-2 border-border bg-muted/60 text-muted-foreground">
                <AlertDescription className="text-xs">{support.reason}</AlertDescription>
              </Alert>
            )}

            <div className="mt-3 flex flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => void loadKeys(issued.jwks_url)}
                disabled={working}
              >
                {busy === "jwks" ? "Fetching…" : "1 · Fetch the key set"}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => keys && void verifyHere(token, keys)}
                disabled={working || !keys || support?.supported === false}
                aria-describedby="resource-verify-note"
              >
                {busy === "verify" ? "Verifying…" : "2 · Verify in this browser"}
              </Button>
            </div>

            <p id="resource-verify-note" className="mt-2 text-xs text-muted-foreground">
              {keys
                ? "Step 2 makes no request of any kind. The keys are already here."
                : "Step 2 needs the keys first — it will not fetch them for you."}
            </p>

            {keysError && (
              <p className="mt-2 text-xs text-warning-foreground">
                The key set could not be read: {keysError}
              </p>
            )}

            {keys && (
              <div className="mt-2">
                <p className="text-xs text-muted-foreground">
                  {keys.length === 1 ? "1 key" : `${keys.length} keys`} fetched at {keysAt} from{" "}
                  <span className="break-all font-mono">{issued.jwks_url}</span>
                </p>
                <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
                  {keys.map((key, index) => (
                    <div key={typeof key.kid === "string" ? key.kid : index} className="contents">
                      <dt className="break-all font-mono text-foreground">
                        {typeof key.kid === "string" ? key.kid : "(no kid)"}
                      </dt>
                      <dd className="font-mono text-muted-foreground">
                        {[key.kty, key.crv, key.alg].filter(Boolean).join(" · ")}
                      </dd>
                    </div>
                  ))}
                </dl>
              </div>
            )}

            <div aria-live="polite">
              {local && (
                <div
                  className={cn(
                    "mt-3 rounded-md border px-3 py-2 text-sm",
                    localVerdictStyle(local.verdict.kind),
                  )}
                >
                  <p className="font-medium">
                    Your browser · {LOCAL_VERDICT_LABELS[local.verdict.kind]}
                  </p>
                  <p className="mt-1 text-xs opacity-90">{describeVerdict(local.verdict)}</p>
                  <p className="mt-1 text-xs opacity-75">
                    Computed at {local.at} against the key set you fetched at {keysAt}. No
                    request was made.
                  </p>
                </div>
              )}
            </div>

            {/*
              Saying "this ran locally" is still us saying it. These are the
              two checks that do not depend on believing the page, which is
              why they are spelled out rather than implied.
            */}
            <p className="mt-3 text-xs text-muted-foreground">
              Two ways to check that for yourself, rather than take it from us: open your
              browser&apos;s network panel and watch it stay silent while you press step 2 — or
              switch your network off entirely and press it anyway. The answer is the same
              offline, which is the whole capability this scenario is about: a resource server
              validates a token without calling the issuer.
            </p>
            <p className="mt-2 text-xs text-muted-foreground">
              Or skip our buttons altogether —{" "}
              <code className="break-all font-mono text-foreground">
                await authkestra.verify(&quot;&lt;paste a token&gt;&quot;)
              </code>{" "}
              in the console runs the same function, on a token of your own, against the keys
              already in this tab.
            </p>
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
            {/* Named, so it sits beside "Your browser · …" as one of two
                answers rather than as the answer. */}
            <p className="font-medium">
              Our API · {result.status} · {VERDICT_LABELS[result.verdict] ?? result.verdict}
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
