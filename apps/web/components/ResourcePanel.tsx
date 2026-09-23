"use client";

import { useCallback, useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import type { Forgery, IssuedToken } from "@playground/api-types";
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
import { Select } from "@/components/ui/select";

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
 * Every named way to mint a token that is meant to fail one specific check,
 * plus an honest one. What the "Try one of ours:" select offers.
 */
export const FORGERIES: { kind: Forgery; label: string }[] = [
  { kind: "unknown_kid", label: "Unpublished key" },
  { kind: "bad_signature", label: "Real `kid`, wrong key" },
  { kind: "untrusted_issuer", label: "Untrusted issuer" },
  { kind: "wrong_audience", label: "Another service" },
  { kind: "expired", label: "Expired" },
  { kind: "missing_kid", label: "No `kid`" },
];

/** Everything "Try one of ours:" can mint: an honest token, or one of six built to fail. */
export type Choice = "valid" | Forgery;

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
 * Two fields, always editable, answer the actual question a visitor has:
 * does this token check out against this key set. The six named forgeries
 * came back as "Try one of ours:" — a mint-and-fill convenience, not a mode —
 * because typing a broken token by hand is the wrong first ask of a visitor
 * who has never seen one. Picking a preset resets the JWKS field to this
 * deployment's own on purpose: every preset is signed by our key under our
 * `kid`, so verifying it against some other key set a visitor had typed in
 * would show `unknown_kid` for nearly all of them, regardless of which one
 * was picked — masking the specific thing each is named for behind a more
 * basic "wrong key set entirely" answer. Editing the JWKS field back to
 * something else afterward is still one keystroke away, and answers a
 * different, equally honest question: what does *this* key set make of a
 * token that is really ours.
 *
 * Nothing is minted until a visitor actually picks a preset — there is no
 * token pre-seeded on load — and picking one only fills the fields. Verify
 * is a separate press, always: minting a real example is one server call a
 * preset cannot avoid, but checking it is a second, deliberate action, not
 * something a dropdown should trigger on its own.
 */
export default function ResourcePanel({ scenarioId, onDemoDisabled }: Props) {
  const [jwksUrl, setJwksUrl] = useState(DEFAULT_JWKS_URL);
  const [token, setToken] = useState("");
  const [banner, setBanner] = useState<string | null>(null);

  // Reset to "" the instant it fires (see the `Select` below), so it is
  // always ready to trigger again — including picking the same preset twice
  // in a row, which a `value`-bound select would otherwise ignore as a
  // no-op change.
  const [presetChoice, setPresetChoice] = useState<Choice | "">("");

  const [keys, setKeys] = useState<Jwk[] | null>(null);
  const [keysUrl, setKeysUrl] = useState<string | null>(null);
  const [keysAt, setKeysAt] = useState<string | null>(null);
  const [keysError, setKeysError] = useState<string | null>(null);
  const [local, setLocal] = useState<{ verdict: LocalVerdict; at: string } | null>(null);
  const [support, setSupport] = useState<Ed25519Support | null>(null);

  // "minting" only happens from the preset select; "fetching" and
  // "verifying" are the two things Verify itself always does, in that
  // order — this is what keeps the fetch-then-verify separation from #76
  // visible without a checklist widget to hold it: the button's own label
  // says which one is happening right now.
  const [phase, setPhase] = useState<"idle" | "minting" | "fetching" | "verifying">("idle");
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

  /** Mints a token via the API: an honest one, or one built to fail a check. */
  async function mintToken(action: "issue" | "forge", kind?: Forgery): Promise<IssuedToken | null> {
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
    return res.data;
  }

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

  /**
   * The one thing pressing Verify does: fetch the key set if this field's
   * URL is not the one already cached, then verify. Picking a preset no
   * longer chains into this — it only fills the fields (see `runPreset`) —
   * so this has exactly one caller now, the button below.
   */
  async function verify() {
    if (busy || !token.trim() || !jwksUrl.trim()) return;
    const tokenValue = token.trim();
    const jwksUrlValue = jwksUrl.trim();
    setBanner(null);
    setLocal(null);

    let activeKeys = keys;
    if (shouldRefetchKeys(keysUrl, jwksUrlValue)) {
      setPhase("fetching");
      activeKeys = await loadKeys(jwksUrlValue);
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
    const verdict = await verifyTokenSignature(tokenValue, activeKeys);
    setLocal({ verdict, at: new Date().toLocaleTimeString() });
    setPhase("idle");
  }

  /**
   * Mints the chosen preset, then points both fields at it — the JWKS field
   * included, and unconditionally: see the module doc for why a preset that
   * left the JWKS field alone would mostly just report `unknown_kid` instead
   * of the thing it is named for.
   */
  async function runPreset(choice: Choice) {
    if (busy) return;
    setBanner(null);
    setLocal(null);
    setKeysError(null);
    setPhase("minting");
    const minted = await mintToken(choice === "valid" ? "issue" : "forge", choice === "valid" ? undefined : choice);
    setPhase("idle");
    if (!minted) return;
    // Fills the fields and stops there — minting is the one server call a
    // real example needs, but picking an example is not the same thing as
    // asking to check it. Verify is a separate, deliberate press.
    setToken(minted.token);
    setJwksUrl(DEFAULT_JWKS_URL);
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
        browser, against a key set that may not even be ours. The JWKS field starts pointed
        at this deployment&apos;s own; pick an example below or paste a token of your own,
        then press Verify.
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

      <div className="flex flex-wrap items-center justify-end gap-2">
        <Label htmlFor="resource-preset" className="text-xs text-muted-foreground">
          Try one of ours:
        </Label>
        <Select
          id="resource-preset"
          // Matches the Verify button below in both dimensions: `h-8` is the
          // native `<select>`'s own default height (`h-9`), overridden to
          // line up with `Button`'s `size="sm"`, and the width is the same
          // literal class as the button's, not just a similar-looking one.
          className="h-8 w-[200px]"
          value={presetChoice}
          disabled={busy}
          onChange={(e) => {
            const value = e.target.value as Choice | "";
            // Reset immediately, not after the mint resolves — see the state
            // declaration for why.
            setPresetChoice("");
            if (value) void runPreset(value);
          }}
        >
          <option value="" disabled hidden>
            Pick an example…
          </option>
          <option value="valid">A valid token</option>
          <optgroup label="Intentionally broken">
            {FORGERIES.map((f) => (
              <option key={f.kind} value={f.kind}>
                {f.label}
              </option>
            ))}
          </optgroup>
        </Select>
        {phase === "minting" && (
          <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Loader2 className="size-3.5 animate-spin" strokeWidth={2} aria-hidden />
            Minting…
          </span>
        )}
      </div>

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
        <div className="flex items-center justify-between gap-2">
          <Label htmlFor="resource-token" className="text-xs text-muted-foreground">
            Token
          </Label>
          {/*
            A secondary affordance, not an alternative: jwt.io's own
            documented algorithm list for verifying a signature has no EdDSA,
            which is the only thing this deployment signs with — it can
            decode what is here, never check it. The fields above are the
            only thing that can. `#token=` is jwt.io's own current deep-link
            fragment, and a token in a fragment is never sent over the wire,
            so nothing here reaches a third party by clicking it.
          */}
          {token.trim() && (
            <a
              href={`https://jwt.io/#token=${encodeURIComponent(token.trim())}`}
              target="_blank"
              rel="noreferrer"
              className="text-xs font-medium text-foreground underline underline-offset-2"
            >
              Decode on jwt.io ↗
            </a>
          )}
        </div>
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

      <div className="flex justify-end">
        <Button
          type="button"
          size="sm"
          // Same literal width as the preset `Select` above — `size="sm"`
          // already matches its overridden `h-8`.
          className="w-[200px]"
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
