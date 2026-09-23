"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Check, ChevronRight, CircleDashed, Loader2, MinusCircle } from "lucide-react";
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
import { CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";

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

/**
 * Every forgery the API can mint, and what each one is for.
 *
 * Used to be six buttons; now they are six `<option>`s in one native
 * `<select>`, grouped under an "intentionally broken" `<optgroup>` (see
 * [`Choice`]). The kind and the label did not need to change for that — an
 * option's value is a string either way.
 */
export const FORGERIES: { kind: Forgery; label: string }[] = [
  { kind: "unknown_kid", label: "Unpublished key" },
  { kind: "bad_signature", label: "Real `kid`, wrong key" },
  { kind: "untrusted_issuer", label: "Untrusted issuer" },
  { kind: "wrong_audience", label: "Another service" },
  { kind: "expired", label: "Expired" },
  { kind: "missing_kid", label: "No `kid`" },
];

/**
 * What "Try:" offers beyond the six forgeries: an honest token, and no token
 * at all. Kept as a record rather than inline strings so a test can pin the
 * wording the same way [`FORGERIES`]'s labels are pinned.
 */
export const FIXED_CHOICE_LABELS: Record<"valid" | "none", string> = {
  valid: "A valid token",
  none: "No token",
};

/**
 * Everything the "Try:" dropdown can be set to: mint an honest token, mint one
 * of the six forgeries, or send no token at all. One value, one control — the
 * three top buttons and the six forgery buttons this replaces were all
 * answering the same question ("what should we present to the route?"), so
 * they are one choice now instead of nine.
 */
export type Choice = "valid" | "none" | Forgery;

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
 * The three things one press of "Run it" does, in order. This is the part of
 * #76 that survives the redesign: the JWKS fetch stays a distinct, awaited
 * step rather than folding into verification, so it can still be watched
 * happening rather than only trusted to have happened. `RunSteps` is what
 * makes that visible without a button for it — a live checklist instead of a
 * second click.
 */
export type StepState = "pending" | "active" | "done" | "error" | "skipped";

export interface RunSteps {
  call: StepState;
  fetchKeys: StepState;
  verify: StepState;
}

/** The label beside each step's checkbox, in the order they run. */
export const RUN_STEP_LABELS: Record<keyof RunSteps, string> = {
  call: "Called the API",
  fetchKeys: "Fetched the key set (your browser)",
  verify: "Verified the signature (your browser)",
};

const PENDING_STEPS: RunSteps = { call: "pending", fetchKeys: "pending", verify: "pending" };

/**
 * Whether the key set cached from a previous run can be reused, or a fresh
 * one is needed for this run.
 *
 * Pulled out as its own pure function because it is the one piece of the
 * one-click chain with a rule worth pinning in a test without rendering
 * anything: "same issuer, same tab, don't ask twice." A visitor trying every
 * forgery in a row hits the same `jwks_url` each time, so only the first
 * `Run it` should cost a request — this is what stops the other five from
 * costing one too.
 */
export function shouldRefetchKeys(cachedUrl: string | null, targetUrl: string): boolean {
  return cachedUrl !== targetUrl;
}

/**
 * The same two functions the run button calls, put somewhere a visitor can
 * reach them without our UI in the way.
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
  /** Whatever the last run brought back. */
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
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ProtectedCall | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [token, setToken] = useState("");
  // The request the current result came from. Without it the panel only
  // asserts an outcome; "called with no token, got a 401" is a claim until you
  // can see the header that was, or was not, sent.
  const [sentHeader, setSentHeader] = useState<string | null>(null);

  // What "Try:" is set to. Defaults to the honest token, because that is the
  // one press a first-time visitor needs before any of the forgeries mean
  // anything.
  const [choice, setChoice] = useState<Choice>("valid");

  // Null until the first `Run it`. The checklist below renders nothing until
  // this exists, which is what keeps an unopened panel from showing three
  // hollow circles for steps nobody asked for yet.
  const [steps, setSteps] = useState<RunSteps | null>(null);

  // The visitor's own half of the scenario. `keys` is what they fetched, kept
  // across runs on purpose: fetch once per issuer, then verify as many tokens
  // as you like — see `shouldRefetchKeys`.
  const [keys, setKeys] = useState<Jwk[] | null>(null);
  const [keysUrl, setKeysUrl] = useState<string | null>(null);
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

  function headerFor(value: string | null): string {
    const trimmed = value?.trim();
    return trimmed
      ? `Authorization: Bearer ${trimmed.length > 48 ? `${trimmed.slice(0, 48)}…` : trimmed}`
      : "(no Authorization header)";
  }

  /** Mints an honest token, or one built to fail one specific check. */
  async function mint(action: "issue" | "forge", kind?: Forgery): Promise<IssuedToken | null> {
    setBanner(null);
    const res = await scenarioAction<IssuedToken>(scenarioId, action, kind ? { kind } : {});
    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return null;
      }
      setBanner(
        res.error.kind === "rate_limited" || res.error.kind === "http_error"
          ? res.error.detail
          : "Could not reach the API to mint a token.",
      );
      return null;
    }
    setIssued(res.data);
    setToken(res.data.token);
    setResult(null);
    setSentHeader(null);
    // A verdict about the previous token would be actively misleading beside
    // this one. The fetched key set stays, because it is about the issuer.
    setLocal(null);
    return res.data;
  }

  /** Calls the protected route with `value` as the bearer token, or none. */
  async function call(value: string | null): Promise<ProtectedCall | null> {
    setBanner(null);
    setSentHeader(headerFor(value));
    const res = await scenarioAction<ProtectedCall>(scenarioId, "call", {
      token: value ?? undefined,
    });
    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return null;
      }
      setBanner(
        res.error.kind === "rate_limited" || res.error.kind === "http_error"
          ? res.error.detail
          : "Could not reach the API.",
      );
      return null;
    }
    setResult(res.data);
    return res.data;
  }

  /** Fetches a key set. **The only step here that touches the network.** */
  const loadKeys = useCallback(async (url: string): Promise<Jwk[] | null> => {
    setKeysError(null);
    // The verdict names the key set it was computed against, so it cannot
    // outlive a fetch of a different one.
    setLocal(null);
    try {
      const fetched = await fetchJwks(url);
      setKeys(fetched);
      setKeysUrl(url);
      setKeysAt(new Date().toLocaleTimeString());
      return fetched;
    } catch (error) {
      setKeys(null);
      setKeysUrl(null);
      setKeysAt(null);
      setKeysError(error instanceof Error ? error.message : "the key set could not be fetched");
      return null;
    }
  }, []);

  /**
   * The verdict the visitor can check. Deliberately `await`s nothing but
   * WebCrypto: everything it needs is already in the tab.
   */
  const verifyHere = useCallback(async (value: string, fetched: Jwk[]): Promise<LocalVerdict> => {
    const verdict = await verifyTokenSignature(value, fetched);
    setLocal({ verdict, at: new Date().toLocaleTimeString() });
    return verdict;
  }, []);

  /**
   * One press does the whole scenario: mint (or send nothing), call, fetch
   * the key set if this run's issuer is not the one already cached, then
   * verify. `steps` is updated between each `await` rather than all at once
   * at the end, which is what keeps the fetch-then-verify separation from
   * #76 *visible* even though nothing gates it behind a second click anymore.
   */
  async function runIt() {
    if (busy) return;
    setBusy(true);
    setSteps({ ...PENDING_STEPS });
    // A verdict about a previous run's token would be actively misleading
    // beside this run's fresh API result — most visibly when switching to
    // "No token", which skips verification entirely and would otherwise
    // leave the last token's badge sitting there unexplained.
    setLocal(null);

    let activeIssued = issued;
    let activeToken: string | null = null;

    if (choice !== "none") {
      const minted = await mint(choice === "valid" ? "issue" : "forge", choice === "valid" ? undefined : choice);
      if (!minted) {
        setBusy(false);
        setSteps(null);
        return;
      }
      activeIssued = minted;
      activeToken = minted.token;
    }

    setSteps((s) => (s ? { ...s, call: "active" } : s));
    const called = await call(activeToken);
    if (!called) {
      setSteps((s) => (s ? { ...s, call: "error", fetchKeys: "skipped", verify: "skipped" } : s));
      setBusy(false);
      return;
    }
    setSteps((s) => (s ? { ...s, call: "done" } : s));

    // Nothing was sent, so there is nothing to check cryptographically —
    // that is a fact about this run, not a failure of it.
    if (!activeToken || !activeIssued) {
      setSteps((s) => (s ? { ...s, fetchKeys: "skipped", verify: "skipped" } : s));
      setBusy(false);
      return;
    }

    setSteps((s) => (s ? { ...s, fetchKeys: "active" } : s));
    let activeKeys = keys;
    if (shouldRefetchKeys(keysUrl, activeIssued.jwks_url)) {
      activeKeys = await loadKeys(activeIssued.jwks_url);
      if (!activeKeys) {
        setSteps((s) => (s ? { ...s, fetchKeys: "error", verify: "skipped" } : s));
        setBusy(false);
        return;
      }
    }
    // Reachable only if `shouldRefetchKeys` said no fetch was needed, which
    // it says only when `keys` is already the cached set for this issuer —
    // this is here for the type checker, not because it should ever fire.
    if (!activeKeys) {
      setSteps((s) => (s ? { ...s, fetchKeys: "error", verify: "skipped" } : s));
      setBusy(false);
      return;
    }
    setSteps((s) => (s ? { ...s, fetchKeys: "done" } : s));

    if (support?.supported === false) {
      setSteps((s) => (s ? { ...s, verify: "skipped" } : s));
      setBusy(false);
      return;
    }

    setSteps((s) => (s ? { ...s, verify: "active" } : s));
    await verifyHere(activeToken, activeKeys);
    setSteps((s) => (s ? { ...s, verify: "done" } : s));
    setBusy(false);
  }

  /**
   * The manual-tamper path: re-checks whatever is currently in the token
   * textarea — edited or not — without minting anything new. Reuses the same
   * fetch-if-needed-then-verify chain as `runIt`, so editing a byte and
   * pressing this is the same demonstration with one fewer step.
   */
  async function reverifyEdited() {
    if (busy || !issued) return;
    setBusy(true);
    setSteps({ call: "skipped", fetchKeys: "pending", verify: "pending" });

    let activeKeys = keys;
    if (shouldRefetchKeys(keysUrl, issued.jwks_url)) {
      setSteps((s) => (s ? { ...s, fetchKeys: "active" } : s));
      activeKeys = await loadKeys(issued.jwks_url);
      if (!activeKeys) {
        setSteps((s) => (s ? { ...s, fetchKeys: "error", verify: "skipped" } : s));
        setBusy(false);
        return;
      }
    }
    // See the matching guard in `runIt` for why this is unreachable in
    // practice.
    if (!activeKeys) {
      setSteps((s) => (s ? { ...s, fetchKeys: "error", verify: "skipped" } : s));
      setBusy(false);
      return;
    }
    setSteps((s) => (s ? { ...s, fetchKeys: "done" } : s));

    if (support?.supported === false) {
      setSteps((s) => (s ? { ...s, verify: "skipped" } : s));
      setBusy(false);
      return;
    }

    setSteps((s) => (s ? { ...s, verify: "active" } : s));
    await verifyHere(token, activeKeys);
    setSteps((s) => (s ? { ...s, verify: "done" } : s));
    setBusy(false);
  }

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
        issuer&apos;s name and the URL of its published keys. Pick what to present, run it, and
        watch the API and your own browser answer independently.
      </p>

      {banner && (
        <Alert className="border-warning/30 bg-warning/10 text-warning-foreground [&>svg]:text-warning-foreground">
          <AlertDescription>{banner}</AlertDescription>
        </Alert>
      )}

      {support?.supported === false && (
        <Alert className="border-border bg-muted/60 text-muted-foreground">
          <AlertDescription className="text-xs">{support.reason}</AlertDescription>
        </Alert>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Label htmlFor="resource-try" className="text-sm text-muted-foreground">
          Try:
        </Label>
        <Select
          id="resource-try"
          className="w-[240px]"
          value={choice}
          disabled={busy}
          onChange={(e) => setChoice(e.target.value as Choice)}
        >
          <option value="valid">{FIXED_CHOICE_LABELS.valid}</option>
          <option value="none">{FIXED_CHOICE_LABELS.none}</option>
          <optgroup label="Intentionally broken">
            {FORGERIES.map((f) => (
              <option key={f.kind} value={f.kind}>
                {f.label}
              </option>
            ))}
          </optgroup>
        </Select>
        <Button type="button" size="sm" onClick={() => void runIt()} disabled={busy}>
          {busy ? "Running…" : "Run it"}
        </Button>
      </div>

      {steps && (
        <ul aria-live="polite" className="flex flex-col gap-1 text-xs text-muted-foreground">
          {(Object.keys(RUN_STEP_LABELS) as (keyof RunSteps)[]).map((key) => (
            <StepRow key={key} status={steps[key]} label={RUN_STEP_LABELS[key]} />
          ))}
        </ul>
      )}

      {(result || local || steps) && (
        <div aria-live="polite" className="flex flex-col gap-1">
          {sentHeader && (
            <p className="break-all font-mono text-[11px] text-muted-foreground">
              GET /api/protected · {sentHeader}
            </p>
          )}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className={cn("rounded-md border px-3 py-2 text-sm", result ? verdictStyle(result.verdict) : "border-border bg-muted/60 text-muted-foreground")}>
              <p className="font-medium">
                Our API
                {result ? ` · ${result.status} · ${VERDICT_LABELS[result.verdict] ?? result.verdict}` : ""}
              </p>
              {result ? (
                <>
                  <p className="mt-1 whitespace-pre-line text-xs opacity-90">{result.detail}</p>
                  {result.subject && (
                    <p className="mt-1 font-mono text-xs opacity-75">sub: {result.subject}</p>
                  )}
                </>
              ) : (
                <p className="mt-1 text-xs opacity-90">Waiting on the call…</p>
              )}
            </div>

            <div
              className={cn(
                "rounded-md border px-3 py-2 text-sm",
                local ? localVerdictStyle(local.verdict.kind) : "border-border bg-muted/60 text-muted-foreground",
              )}
            >
              <p className="font-medium">
                Your browser
                {local ? ` · ${LOCAL_VERDICT_LABELS[local.verdict.kind]}` : ""}
              </p>
              {local ? (
                <>
                  <p className="mt-1 text-xs opacity-90">{describeVerdict(local.verdict)}</p>
                  <p className="mt-1 text-xs opacity-75">
                    Computed at {local.at} against the key set fetched at {keysAt}. No request
                    was made.
                  </p>
                </>
              ) : (
                <p className="mt-1 text-xs opacity-90">
                  {steps?.verify === "skipped"
                    ? "Nothing to check — no token was sent."
                    : "Waiting on the key set…"}
                </p>
              )}
            </div>
          </div>
          {/*
            Said once, beside both cards rather than before them: a wrong-
            audience or expired token really does verify, and the API refuses
            it on policy. That is agreement, not conflict, and it only reads
            that way once both verdicts are already on screen.
          */}
          {result && local && result.verdict !== "accepted" && local.verdict.kind === "verified" && (
            <p className="text-xs text-muted-foreground">
              Agreement, not conflict: the signature is real, so your browser accepts it — the
              API additionally checks policy (issuer, audience, expiry), which this token fails.
            </p>
          )}
        </div>
      )}

      {keysError && (
        <p className="text-xs text-warning-foreground">The key set could not be read: {keysError}</p>
      )}

      {/*
        Everything below is the falsifiability material from #76 — the token,
        its claims, the raw header, the fetched keys, the offline-proof
        instructions, and the console handle. None of it is load-bearing for a
        visitor who just wants the verdict, so it is one `<details>` rather
        than three-plus standing paragraphs and a JSON dump always on screen.
      */}
      {issued && (
        <details className="group rounded-xl border bg-card text-card-foreground shadow">
          <summary className="flex cursor-pointer list-none items-center gap-2 p-3 text-xs font-medium text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
            <ChevronRight
              aria-hidden
              strokeWidth={1.5}
              className="size-4 shrink-0 transition-transform duration-200 group-open:rotate-90"
            />
            Show the token, its claims, and how to check us
          </summary>
          <CardContent className="flex flex-col gap-3 border-t p-3 pt-3">
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
            <p className="text-xs text-muted-foreground">
              This route accepts <code className="font-mono text-foreground">{issued.audience}</code>{" "}
              in <code className="font-mono">aud</code>, from{" "}
              <code className="font-mono text-foreground">{issued.issuer}</code>.
            </p>

            <div>
              <p className="text-xs text-muted-foreground">
                Decoded from the token below. Nothing here was sent to us, or by us:
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
            </div>

            {header && (
              <div>
                <div className="flex items-center justify-between gap-2">
                  <p className="text-xs text-muted-foreground">Raw header</p>
                  {/*
                    A secondary affordance, not a replacement: jwt.io can show
                    the familiar debugger view, but it does not know this
                    issuer's JWKS, so it cannot tell you whether *this* route
                    would accept the token, and it cannot re-run the tamper
                    check below against our actual key set. The token travels
                    only in the fragment, which browsers never send over the
                    wire, so nothing here is handed to a third party.
                  */}
                  <a
                    href={`https://jwt.io/#debugger-io?token=${encodeURIComponent(token)}`}
                    target="_blank"
                    rel="noreferrer"
                    className="text-xs font-medium text-foreground underline underline-offset-2"
                  >
                    View on jwt.io ↗
                  </a>
                </div>
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
            <p className="text-xs text-muted-foreground">
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

            {keys && (
              <div>
                <p className="text-xs text-muted-foreground">
                  {keys.length === 1 ? "1 key" : `${keys.length} keys`} fetched at {keysAt} from{" "}
                  <span className="break-all font-mono">{keysUrl}</span>
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

            <div>
              <Label className="block text-xs font-normal text-muted-foreground" htmlFor="resource-token">
                Edit before re-checking to see the rest — change a character in the last segment
                for a bad signature, or delete one to make it unreadable. Whatever is here is
                what gets sent.
              </Label>
              <textarea
                id="resource-token"
                value={token}
                onChange={(e) => {
                  setToken(e.target.value);
                  // A verdict about bytes that are no longer in the box is
                  // worse than no verdict: change one character and the badge
                  // above goes away until you ask again.
                  setLocal(null);
                }}
                spellCheck={false}
                rows={3}
                className="mt-1 w-full resize-y break-all rounded-md border border-input bg-transparent p-2 font-mono text-xs text-foreground placeholder:text-muted-foreground"
              />
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="mt-2"
                onClick={() => void reverifyEdited()}
                disabled={busy || support?.supported === false}
              >
                Re-check this token
              </Button>
            </div>

            {/*
              One paragraph, not three. The policy-vs-cryptography distinction
              already has its own line beside the result cards, shown exactly
              when it applies — repeating it here in prose was reading
              material, not new information. What is left is the two things a
              visitor can check without trusting either the page or us.
            */}
            <p className="text-xs text-muted-foreground">
              Check for yourself: the network panel should show exactly one request — the
              key-set fetch — then stay silent while the signature verifies, and the verdict
              should not change if you switch the network off and press &quot;Run it&quot;
              again. Or skip our controls —{" "}
              <code className="break-all font-mono text-foreground">
                await authkestra.verify(&quot;&lt;paste a token&gt;&quot;)
              </code>{" "}
              in the console runs the same function against the keys already in this tab.
            </p>
          </CardContent>
        </details>
      )}
    </div>
  );
}

/** One line of the running checklist: a fixed icon per [`StepState`]. */
function StepRow({ status, label }: { status: StepState; label: string }) {
  return (
    <li className="flex items-center gap-2">
      {status === "done" && <Check className="size-3.5 shrink-0 text-success-foreground" strokeWidth={2} aria-hidden />}
      {status === "active" && (
        <Loader2 className="size-3.5 shrink-0 animate-spin text-foreground" strokeWidth={2} aria-hidden />
      )}
      {status === "pending" && <CircleDashed className="size-3.5 shrink-0 opacity-50" strokeWidth={1.5} aria-hidden />}
      {status === "error" && <MinusCircle className="size-3.5 shrink-0 text-destructive-foreground" strokeWidth={2} aria-hidden />}
      {status === "skipped" && <MinusCircle className="size-3.5 shrink-0 opacity-50" strokeWidth={1.5} aria-hidden />}
      <span
        className={cn(
          status === "done" && "text-foreground",
          status === "error" && "text-destructive-foreground",
        )}
      >
        {label}
        {status === "skipped" && " — not applicable"}
        {status === "error" && " — failed"}
      </span>
    </li>
  );
}
