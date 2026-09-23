"use client";

import { useCallback, useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import type { IssuedToken } from "@playground/api-types";
import { API_BASE, scenarioAction } from "@/lib/api";
import { cn } from "@/lib/cn";
import {
  checkEd25519Support,
  fetchJwks,
  describeVerdict,
  verifyTokenSignature,
  type Ed25519Support,
  type Jwk,
  type LocalVerdict,
} from "@/lib/jwt";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-flow. */
  onDemoDisabled: () => void;
}

/**
 * What to call the verdict, in words rather than the algorithm's vocabulary.
 * Kept exported and pinned by a test because a verdict added upstream in
 * `lib/jwt.ts`'s `LocalVerdict` union that never gets a label here would
 * render as its raw wire name — the kind of gap nobody notices in review.
 */
export const LOCAL_VERDICT_LABELS: Record<LocalVerdict["kind"], string> = {
  verified: "Verified",
  bad_signature: "Bad signature",
  unknown_kid: "Unpublished key",
  missing_kid: "No `kid` header",
  malformed: "Malformed token",
  unsupported_alg: "Unchecked algorithm",
  unsupported: "Cannot be checked here",
};

/**
 * Colour by what the verdict means. Destructive is reserved for a signature
 * that does not verify — someone forged or tampered it — not for every
 * non-"verified" outcome, because "malformed" or "no `kid`" are usually just
 * a visitor mid-edit, not a finding.
 *
 * "Cannot be checked here" is deliberately neutral rather than alarming. A
 * browser without Ed25519, or a key this page cannot import, is not a
 * finding about the token.
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
 * Whether the key set already fetched can be reused, or this URL needs a
 * fresh fetch.
 *
 * Its own function rather than an inline comparison because it is the one
 * rule worth pinning in a test without rendering anything: "same URL as what
 * is already here, don't ask twice." Editing the token and pressing Verify
 * again should not cost a second request against the same key set.
 */
export function shouldRefetchKeys(cachedUrl: string | null, targetUrl: string): boolean {
  return cachedUrl !== targetUrl;
}

/**
 * Where both fields start: this deployment's own key set, published at the
 * same well-known path the resource server itself reads (see
 * `apps/api/src/signing.rs`). A visitor who changes nothing still sees a real
 * verification happen; changing either field is how they check anything else
 * — a key set of their own, or a token that never came from here at all.
 */
const DEFAULT_JWKS_URL = `${API_BASE}/.well-known/jwks.json`;

/**
 * The same function the button calls, put somewhere a visitor can reach it
 * without our UI in the way.
 *
 * `authkestra.verify(token)` in the console is this code with our rendering
 * removed — checked against whichever key set is currently in the JWKS field,
 * with no need to trust that our button did what it says.
 */
export interface ConsoleHandle {
  /** Verifies against the key set already fetched. Makes no request. */
  verify(token: string, keys?: Jwk[]): Promise<LocalVerdict>;
  /** The one call that does touch the network, kept separate on purpose. */
  fetchKeys(url?: string): Promise<Jwk[]>;
  /** Whatever the last fetch brought back. */
  keys: Jwk[] | null;
}

declare global {
  interface Window {
    authkestra?: ConsoleHandle;
  }
}

/**
 * Verify a token's signature against a JWKS — anyone's, not just ours — with
 * nothing sent anywhere but the key-set fetch itself.
 *
 * This used to be a scenario with a "Try:" dropdown, six named ways to mint a
 * broken token, a call to our own protected route, and a collapsed section
 * holding the token's claims and an explanation of why a browser-computed
 * verdict is worth trusting. All of that answered a narrower question than
 * this one does. Two fields and one button answer the actual question a
 * visitor has: does this token check out against this key set. Editing
 * either field, badly or well, is how every one of the old scenario's
 * lessons — an unpublished key, a tampered signature, a malformed token —
 * still happens, just by hand instead of by preset.
 */
export default function ResourcePanel({ scenarioId, onDemoDisabled }: Props) {
  const [jwksUrl, setJwksUrl] = useState(DEFAULT_JWKS_URL);
  const [token, setToken] = useState("");
  const [banner, setBanner] = useState<string | null>(null);

  const [keys, setKeys] = useState<Jwk[] | null>(null);
  const [keysUrl, setKeysUrl] = useState<string | null>(null);
  const [keysAt, setKeysAt] = useState<string | null>(null);
  const [keysError, setKeysError] = useState<string | null>(null);
  const [local, setLocal] = useState<{ verdict: LocalVerdict; at: string } | null>(null);
  const [support, setSupport] = useState<Ed25519Support | null>(null);

  // "fetching" and "verifying" are the only two things a press of the button
  // ever does, in that order — this is what keeps the fetch-then-verify
  // separation from #76 visible without a checklist widget to hold it: the
  // button's own label says which one is happening right now.
  const [phase, setPhase] = useState<"idle" | "fetching" | "verifying">("idle");
  const busy = phase !== "idle";

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

  // The token field defaults to a real, working token rather than a
  // hardcoded example — a hardcoded one would already be expired, since
  // these are short-lived, and a default that fails before anything is
  // pressed teaches the wrong lesson. If this fails (rate-limited, the demo
  // disabled, the API unreachable) the field just stays empty; pasting a
  // token by hand still works.
  useEffect(() => {
    let cancelled = false;
    void scenarioAction<IssuedToken>(scenarioId, "issue", {}).then((res) => {
      if (cancelled) return;
      if (res.ok) setToken(res.data.token);
      else if (res.error.kind === "demo_disabled") onDemoDisabled();
    });
    return () => {
      cancelled = true;
    };
    // Intentionally once per mount: this seeds a starting example, it does
    // not track `scenarioId` changing under a fixed panel instance.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** Fetches a key set. **The only step here that touches the network.** */
  const loadKeys = useCallback(async (url: string): Promise<Jwk[] | null> => {
    setKeysError(null);
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

  async function verify() {
    if (busy || !token.trim() || !jwksUrl.trim()) return;
    setBanner(null);
    setLocal(null);

    let activeKeys = keys;
    if (shouldRefetchKeys(keysUrl, jwksUrl)) {
      setPhase("fetching");
      activeKeys = await loadKeys(jwksUrl);
    }
    // Reachable if the cache was reused (`activeKeys` is still `keys`) and
    // nothing has ever been fetched yet, or if the fetch above just failed.
    if (!activeKeys) {
      setPhase("idle");
      return;
    }

    if (support?.supported === false) {
      setPhase("idle");
      return;
    }

    setPhase("verifying");
    const verdict = await verifyTokenSignature(token, activeKeys);
    setLocal({ verdict, at: new Date().toLocaleTimeString() });
    setPhase("idle");
  }

  // The console handle. Installed on mount and removed on unmount, because a
  // global that outlives the panel that explains it is just litter.
  useEffect(() => {
    if (typeof window === "undefined") return;
    const handle: ConsoleHandle = {
      verify: (value, override) => verifyTokenSignature(value, override ?? keys ?? []),
      fetchKeys: (url) => fetchJwks(url ?? jwksUrl),
      keys,
    };
    window.authkestra = handle;
    return () => {
      if (window.authkestra === handle) delete window.authkestra;
    };
  }, [keys, jwksUrl]);

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-muted-foreground">
        Verify a token&apos;s signature against a published key set — entirely in this
        browser, against a key set that may not even be ours. Starts pointed at this
        deployment&apos;s own key set and a token it just issued you; change either to check
        anything else.
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

      <div>
        <Label htmlFor="resource-jwks-url" className="text-xs text-muted-foreground">
          Key set (JWKS) URL
        </Label>
        <Input
          id="resource-jwks-url"
          value={jwksUrl}
          onChange={(e) => {
            setJwksUrl(e.target.value);
            setLocal(null);
          }}
          spellCheck={false}
          className="mt-1 font-mono text-xs"
        />
      </div>

      <div>
        <Label htmlFor="resource-token" className="text-xs text-muted-foreground">
          Token
        </Label>
        <textarea
          id="resource-token"
          value={token}
          onChange={(e) => {
            setToken(e.target.value);
            setLocal(null);
          }}
          spellCheck={false}
          rows={3}
          placeholder="eyJhbGciOi…"
          className="mt-1 w-full resize-y break-all rounded-md border border-input bg-transparent p-2 font-mono text-xs text-foreground placeholder:text-muted-foreground"
        />
      </div>

      <div>
        <Button
          type="button"
          size="sm"
          onClick={() => void verify()}
          disabled={busy || !token.trim() || !jwksUrl.trim() || support?.supported === false}
        >
          {phase === "fetching" && (
            <Loader2 className="mr-1.5 size-3.5 animate-spin" strokeWidth={2} aria-hidden />
          )}
          {phase === "verifying" && (
            <Loader2 className="mr-1.5 size-3.5 animate-spin" strokeWidth={2} aria-hidden />
          )}
          {phase === "fetching" ? "Fetching the key set…" : phase === "verifying" ? "Verifying…" : "Verify"}
        </Button>
      </div>

      {keysError && (
        <p className="text-xs text-warning-foreground">The key set could not be read: {keysError}</p>
      )}

      {local && (
        <div
          aria-live="polite"
          className={cn("rounded-md border px-3 py-2 text-sm", localVerdictStyle(local.verdict.kind))}
        >
          <p className="font-medium">{LOCAL_VERDICT_LABELS[local.verdict.kind]}</p>
          <p className="mt-1 text-xs opacity-90">{describeVerdict(local.verdict)}</p>
          <p className="mt-1 text-xs opacity-75">
            Computed at {local.at} against the key set fetched at {keysAt} from{" "}
            <span className="break-all font-mono">{keysUrl}</span>. No other request was made.
          </p>
        </div>
      )}
    </div>
  );
}
