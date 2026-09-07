"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { CaptchaVerification, CaptchaWidget, CaptchaWidgets } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-flow. */
  onDemoDisabled: () => void;
  /** Called after every round trip, so the flow log can refetch. */
  onAction?: () => void;
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

/** Stands in for a forged or replayed token — the failure path issue #19 wants demonstrable. */
const FAKE_TOKEN = "forged-token-0000000000000000";

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
  /** reCAPTCHA only: must resolve before `render` is safe to call. */
  ready?: (callback: () => void) => void;
}

/** The one place a third-party global gets assumed to exist on `window`. */
function providerApi(global: ProviderGlobal): CaptchaWidgetApi | undefined {
  return (window as unknown as Record<ProviderGlobal, CaptchaWidgetApi | undefined>)[global];
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

export default function CaptchaPanel({ scenarioId, onDemoDisabled, onAction }: Props) {
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
      onAction?.();

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
            onAction={onAction}
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
  onAction,
}: {
  widget: CaptchaWidget;
  scenarioId: string;
  onDemoDisabled: () => void;
  onAction?: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const widgetIdRef = useRef<string | number | null>(null);

  const [token, setToken] = useState<string | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [result, setResult] = useState<CaptchaVerification | null>(null);

  const config = PROVIDER_SCRIPTS[widget.provider];

  useEffect(() => {
    if (!config) return;
    let cancelled = false;

    loadScript(config.src)
      .then(() => {
        if (cancelled) return;
        const api = providerApi(config.global);
        if (!api || !containerRef.current) {
          setRenderError(`The ${widget.label} script loaded but didn't expose its API.`);
          return;
        }

        const doRender = () => {
          if (cancelled || !containerRef.current) return;
          widgetIdRef.current = api.render(containerRef.current, {
            sitekey: widget.site_key,
            callback: (t) => {
              if (!cancelled) setToken(t);
            },
            "error-callback": () => {
              if (!cancelled) setRenderError(`${widget.label} reported an error. Try refreshing the page.`);
            },
            "expired-callback": () => {
              if (!cancelled) setToken(null);
            },
          });
        };

        // reCAPTCHA needs `ready()` before it will render; the other two don't expose it.
        if (typeof api.ready === "function") {
          api.ready(doRender);
        } else {
          doRender();
        }
      })
      .catch(() => {
        if (!cancelled) setRenderError(`Could not load the ${widget.label} script.`);
      });

    return () => {
      cancelled = true;
    };
  }, [config, widget.label, widget.provider, widget.site_key]);

  const verify = useCallback(
    async (tokenToSend: string, { consumesWidget }: { consumesWidget: boolean }) => {
      setVerifying(true);
      setBanner(null);

      const res = await scenarioAction<CaptchaVerification>(scenarioId, "verify", {
        provider: widget.provider,
        token: tokenToSend,
      });

      setVerifying(false);
      onAction?.();

      if (!res.ok) {
        if (res.error.kind === "demo_disabled") return onDemoDisabled();
        setBanner(res.error.kind === "rate_limited" ? res.error.detail : "The provider could not verify this token.");
        return;
      }

      // verified: false is a normal outcome here, not an error — render it inline.
      setResult(res.data);

      if (consumesWidget) {
        // Tokens are single-use, so the widget needs to hand out a fresh one.
        setToken(null);
        if (config && widgetIdRef.current !== null) {
          providerApi(config.global)?.reset(widgetIdRef.current);
        }
      }
    },
    [scenarioId, widget.provider, onDemoDisabled, onAction, config],
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

      <div className="mt-3 flex flex-wrap gap-2">
        <button
          type="button"
          onClick={() => void verify(token ?? "", { consumesWidget: true })}
          disabled={!token || verifying}
          className="rounded-md bg-emerald-500 px-3 py-1.5 text-sm font-semibold text-slate-950 transition hover:bg-emerald-400 disabled:cursor-not-allowed disabled:bg-emerald-500/50"
        >
          {verifying ? "Verifying…" : "Verify this token"}
        </button>

        <button
          type="button"
          onClick={() => void verify(FAKE_TOKEN, { consumesWidget: false })}
          disabled={verifying}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Send a token that cannot pass
        </button>
      </div>

      {banner && <p className="mt-2 text-xs text-amber-400">{banner}</p>}

      <div aria-live="polite" className="mt-2">
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
