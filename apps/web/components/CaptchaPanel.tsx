"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { CaptchaVerification, CaptchaWidget, CaptchaWidgets } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-flow. */
  onDemoDisabled: () => void;
}

type ProviderGlobal = "turnstile" | "hcaptcha" | "grecaptcha";

/**
 * Script URL + the global it installs, per provider.
 *
 * Deliberately a frontend constant rather than server-supplied: these are
 * fixed properties of each provider, not of this deployment, and keeping them
 * here means no server string is ever interpolated into a `<script src>`.
 * Mirrors the same reasoning on `CaptchaWidget` in the Rust scenario.
 */
export const PROVIDER_SCRIPTS: Record<string, { src: string; global: ProviderGlobal }> = {
  turnstile: {
    src: "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit",
    global: "turnstile",
  },
  hcaptcha: {
    src: "https://js.hcaptcha.com/1/api.js?render=explicit",
    global: "hcaptcha",
  },
  recaptcha: {
    src: "https://www.google.com/recaptcha/api.js?render=explicit",
    global: "grecaptcha",
  },
};

/** Whatever a thrown value has to say for itself. */
function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Long tokens are unreadable in full and the point is recognisability, not the value. */
function truncate(value: string, keep = 48): string {
  return value.length <= keep ? value : `${value.slice(0, keep)}… (${value.length} chars)`;
}

/** A failed captcha is the expected outcome of the failure button, not an error — amber, not red. */
export function verdictStyle(verified: boolean): string {
  return verified
    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
    : "border-amber-500/30 bg-amber-500/10 text-amber-300";
}

export function verdictLabel(verified: boolean): string {
  return verified ? "Verified" : "Not verified";
}

/** Narrow view of what all three widget scripts install on `window`. Same shape, different global. */
interface CaptchaWidgetApi {
  render(
    container: HTMLElement,
    params: {
      sitekey: string;
      callback: (token: string) => void;
      "error-callback"?: () => void;
      "expired-callback"?: () => void;
    },
  ): string | number;
  reset(widgetId?: string | number): void;
}

/** The one place a third-party global gets assumed to exist on `window`. */
function providerApi(global: ProviderGlobal): CaptchaWidgetApi | undefined {
  return (window as unknown as Record<ProviderGlobal, CaptchaWidgetApi | undefined>)[global];
}

/**
 * Wait until the provider's global can actually render.
 *
 * A loaded script is not a ready one. reCAPTCHA installs `grecaptcha` before
 * `grecaptcha.render` exists, so calling render on script load produced a
 * permanently blank widget — the failure this replaces. Waiting on `render`
 * itself works for all three and needs no per-provider special case, which the
 * previous `grecaptcha.ready()` branch did (and which the other two never
 * expose, so it silently only covered one of them).
 */
const READY_TIMEOUT_MS = 10_000;

async function awaitRenderable(global: ProviderGlobal): Promise<CaptchaWidgetApi> {
  const deadline = Date.now() + READY_TIMEOUT_MS;
  for (;;) {
    const api = providerApi(global);
    if (api && typeof api.render === "function") return api;
    if (Date.now() > deadline) {
      throw new Error(`window.${global}.render did not appear within ${READY_TIMEOUT_MS}ms`);
    }
    await new Promise((r) => setTimeout(r, 50));
  }
}

// Module-level so a remount (or two panels on the same page) never fetches
// the same provider's script twice.
const scriptCache = new Map<string, Promise<void>>();

function loadScript(src: string): Promise<void> {
  const cached = scriptCache.get(src);
  if (cached) return cached;

  const promise = new Promise<void>((resolve, reject) => {
    const script = document.createElement("script");
    script.src = src;
    script.async = true;
    script.onload = () => resolve();
    script.onerror = () => reject(new Error(`could not load ${src}`));
    document.head.appendChild(script);
  });
  scriptCache.set(src, promise);
  return promise;
}

export default function CaptchaPanel({ scenarioId, onDemoDisabled }: Props) {
  const [widgets, setWidgets] = useState<CaptchaWidget[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [banner, setBanner] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function fetchWidgets() {
      setLoading(true);
      setBanner(null);
      const res = await scenarioAction<CaptchaWidgets>(scenarioId, "widget");
      if (cancelled) return;
      setLoading(false);

      if (!res.ok) {
        if (res.error.kind === "demo_disabled") return onDemoDisabled();
        setBanner(res.error.kind === "rate_limited" ? res.error.detail : "Could not load the captcha widgets.");
        return;
      }
      setWidgets(res.data.widgets);
    }

    void fetchWidgets();
    return () => {
      cancelled = true;
    };
    // Only on mount (and if the scenario itself changes) — the widget list
    // doesn't change on its own, so nothing else here should retrigger it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scenarioId]);

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-slate-400">
        One implementation, three providers — solve each widget below, then verify its
        token against the server. The failure button sends a token no provider issued,
        so you can see what a forged or replayed token looks like to the check.
      </p>
      <p className="text-xs text-slate-400">
        In a real application this check is not a step of its own: it goes at the top of
        the handler you want protected, before any lookup and before any work. The diff
        in step 1 names the route it guards.
      </p>

      {banner && (
        <p className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-300">
          {banner}
        </p>
      )}

      {loading && <p className="text-xs text-slate-400">Loading widgets…</p>}

      {!loading && widgets?.length === 0 && (
        <p className="text-xs text-slate-400">
          No captcha provider is both selected and configured with keys on this deployment.
        </p>
      )}

      <div className="flex flex-col gap-3">
        {widgets?.map((widget) => (
          <CaptchaWidgetCard
            key={widget.provider}
            widget={widget}
            scenarioId={scenarioId}
            onDemoDisabled={onDemoDisabled}
          />
        ))}
      </div>
    </div>
  );
}

function CaptchaWidgetCard({
  widget,
  scenarioId,
  onDemoDisabled,
}: {
  widget: CaptchaWidget;
  scenarioId: string;
  onDemoDisabled: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const widgetIdRef = useRef<string | number | null>(null);

  const [token, setToken] = useState("");
  const [sent, setSent] = useState<string | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [result, setResult] = useState<CaptchaVerification | null>(null);

  const config = PROVIDER_SCRIPTS[widget.provider];

  useEffect(() => {
    if (!config) return;
    let cancelled = false;

    // Three failure modes, three messages. They used to share one `.catch`,
    // which reported "could not load the script" for a `render()` that threw —
    // so a rejected site key read as a network problem and sent us looking in
    // the wrong place entirely.
    (async () => {
      try {
        await loadScript(config.src);
      } catch {
        if (!cancelled) {
          setRenderError(
            `Could not fetch the ${widget.label} script. An ad blocker or strict ` +
              `tracking protection will do this; so will being offline.`,
          );
        }
        return;
      }

      let api: CaptchaWidgetApi;
      try {
        api = await awaitRenderable(config.global);
      } catch (e) {
        if (!cancelled) {
          setRenderError(
            `The ${widget.label} script loaded but never became usable (${message(e)}).`,
          );
        }
        return;
      }

      if (cancelled || !containerRef.current) return;

      try {
        widgetIdRef.current = api.render(containerRef.current, {
          sitekey: widget.site_key,
          callback: (t) => {
            if (!cancelled) setToken(t);
          },
          "error-callback": () => {
            if (!cancelled) {
              setRenderError(
                `${widget.label} refused the widget. The usual cause is a site key ` +
                  `whose registered hostnames do not include this one.`,
              );
            }
          },
          "expired-callback": () => {
            if (!cancelled) setToken("");
          },
        });
      } catch (e) {
        // Most often a site key the provider will not accept here — which is
        // worth saying, rather than blaming the network.
        if (!cancelled) {
          setRenderError(`${widget.label} would not render this site key: ${message(e)}`);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [config, widget.label, widget.provider, widget.site_key]);

  const verify = useCallback(
    async (tokenToSend: string) => {
      setVerifying(true);
      setBanner(null);
      // Keep what was sent, so the outcome is shown next to its own cause
      // rather than asserted on its own.
      setSent(tokenToSend);

      const res = await scenarioAction<CaptchaVerification>(scenarioId, "verify", {
        provider: widget.provider,
        token: tokenToSend,
      });

      setVerifying(false);

      if (!res.ok) {
        if (res.error.kind === "demo_disabled") return onDemoDisabled();
        // The server's own words, not a paraphrase. Sending an empty box is a
        // legitimate thing to try — the 400 explaining it is the demonstration,
        // so replacing it with "something went wrong" would throw away the
        // only part that teaches anything.
        setBanner(
          res.error.kind === "rate_limited" || res.error.kind === "http_error"
            ? res.error.detail
            : "Could not reach the API to verify that token.",
        );
        setResult(null);
        return;
      }

      // verified: false is a normal outcome here, not an error — render inline.
      setResult(res.data);

      // A token the provider accepted has been spent, so the widget owes us a
      // fresh one. A rejected one leaves the box alone: that is the evidence.
      if (res.data.verified) {
        setToken("");
        if (config && widgetIdRef.current !== null) {
          providerApi(config.global)?.reset(widgetIdRef.current);
        }
      }
    },
    [scenarioId, widget.provider, onDemoDisabled, config],
  );

  if (!config) {
    return (
      <div className="rounded-md border border-slate-800 bg-slate-900/60 p-3">
        <p className="text-xs text-amber-400">Unknown captcha provider &quot;{widget.provider}&quot;.</p>
      </div>
    );
  }

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/60 p-3">
      <p className="text-sm font-medium text-slate-200">{widget.label}</p>

      <div ref={containerRef} className="mt-2 min-h-[78px]" />

      {renderError && <p className="mt-2 text-xs text-amber-400">{renderError}</p>}

      <label className="mt-3 block text-xs text-slate-400" htmlFor={`captcha-token-${widget.provider}`}>
        The token the widget produced. Edit a character to see a forged one
        rejected, or empty the box to see what arrives with no token at all —
        whatever is here is exactly what gets sent.
      </label>
      <textarea
        id={`captcha-token-${widget.provider}`}
        value={token}
        onChange={(e) => setToken(e.target.value)}
        spellCheck={false}
        rows={3}
        placeholder="Solve the widget above, or paste a token to try"
        className="mt-1 w-full resize-y break-all rounded border border-slate-700 bg-slate-950 p-2 font-mono text-xs text-slate-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-400"
      />

      <div className="mt-2 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void verify(token)}
          disabled={verifying}
          className="rounded-md bg-emerald-500 px-3 py-1.5 text-sm font-semibold text-slate-950 transition hover:bg-emerald-400 disabled:cursor-not-allowed disabled:bg-emerald-500/50"
        >
          {verifying ? "Verifying…" : "Verify this token"}
        </button>
      </div>

      {banner && <p className="mt-2 text-xs text-amber-400">{banner}</p>}

      <div aria-live="polite" className="mt-2">
        {sent !== null && (
          <p className="mb-1 break-all font-mono text-[11px] text-slate-400">
            sent: {sent === "" ? "(nothing — no token field)" : truncate(sent)}
          </p>
        )}
        {result && (
          <div className={`rounded-md border px-3 py-2 text-sm ${verdictStyle(result.verified)}`}>
            <p className="font-medium">
              {result.label} · {verdictLabel(result.verified)}
            </p>
            <p className="mt-1 text-xs opacity-90">{result.detail}</p>
          </div>
        )}
      </div>
    </div>
  );
}
