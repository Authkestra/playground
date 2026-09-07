"use client";

import { Fragment, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { DemoConfig, OAuthMode, ScenarioSpec } from "@playground/api-types";
import { loginUrl, type OAuthReturn } from "@/lib/oauth";
import { isControlValueActive } from "@/components/ScenarioPanel";
import TotpPanel from "@/components/TotpPanel";
import PasskeysPanel from "@/components/PasskeysPanel";
import ResourcePanel from "@/components/ResourcePanel";
import CaptchaPanel from "@/components/CaptchaPanel";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  oauthReturn: OAuthReturn | null;
  onDismissOauthReturn: () => void;
  onDemoDisabled: () => void;
  onBack: () => void;
  onContinue: () => void;
}

// Brand-ish surfaces, so these carry light text regardless of the dark theme.
// A blanket light->dark recolour got this wrong once: it turned white label
// text dark, leaving near-black text on a near-black GitHub button.
const PROVIDER_STYLES: Record<string, string> = {
  github: "bg-slate-100 text-slate-900 hover:bg-white",
  google: "border border-slate-600 bg-slate-800 text-slate-100 hover:bg-slate-700",
  discord: "bg-indigo-500 text-white hover:bg-indigo-400",
};

const PROVIDER_FALLBACK_STYLE =
  "border border-slate-700 bg-slate-900 text-slate-200 hover:bg-slate-800";

/**
 * Scenarios with a panel in the sign-in step, in the order they appear.
 *
 * Bot protection leads: a captcha guards the form, so it belongs in front of
 * the methods rather than after them.
 */
export const PANEL_ORDER = ["captcha", "oauth", "passkeys", "totp", "resource"] as const;

/** Switched on by the visitor *and* usable on this deployment. */
export function isScenarioLive(
  scenarios: ScenarioSpec[],
  config: DemoConfig | null,
  id: string,
): boolean {
  const spec = scenarios.find((s) => s.id === id);
  // Both halves matter. A kill-switched scenario used to render its panel
  // anyway, and then every button inside it answered 503.
  return !!spec && spec.available !== false && isControlValueActive(config?.scenarios?.[id]);
}

/**
 * Which panels to show, in order.
 *
 * Pure and exported because this is where the bug was: the "is anything on?"
 * check listed every panel except the resource server, so turning on only the
 * protected route showed "no sign-in method is turned on yet" while its panel
 * sat one branch away, never rendered. Deriving the list from `PANEL_ORDER`
 * makes that unrepresentable — a panel cannot render without counting towards
 * the empty state, and the dividers between them cannot disagree with what is
 * actually on either side.
 */
export function visiblePanels(
  scenarios: ScenarioSpec[],
  config: DemoConfig | null,
  oauthOptionCount: number,
): string[] {
  return PANEL_ORDER.filter((id) => {
    if (!isScenarioLive(scenarios, config, id)) return false;
    // A provider can be selected and no longer offered — credentials pulled
    // from the deployment — so OAuth needs a button to show, not a selection.
    if (id === "oauth") return oauthOptionCount > 0;
    return true;
  });
}

/** Whether any of the live panels is actually a way to sign in. */
export function hasSignInMethod(visible: string[]): boolean {
  return visible.some((id) => id === "oauth" || id === "passkeys" || id === "totp");
}

export default function StepSignIn({
  scenarios,
  config,
  oauthReturn,
  onDismissOauthReturn,
  onDemoDisabled,
  onBack,
  onContinue,
}: Props) {
  const [oauthMode, setOauthMode] = useState<OAuthMode>("session");

  const oauthScenario = scenarios.find((s) => s.id === "oauth");
  const oauthValue = config?.scenarios.oauth;
  const selectedProviders = oauthValue?.kind === "select_many" ? oauthValue.selected : [];
  const oauthOptions =
    oauthScenario?.control.kind === "select_many" ? oauthScenario.control.options : [];
  const activeOauthOptions = oauthOptions.filter((o) => selectedProviders.includes(o.id));

  const visible = visiblePanels(scenarios, config, activeOauthOptions.length);
  const signInMethodOn = hasSignInMethod(visible);

  const renderPanel = (id: string): ReactNode => {
    switch (id) {
      case "captcha":
        return (
          <div className="rounded-md border border-slate-800 p-4">
            <h4 className="mb-2 text-sm font-medium text-slate-200">Bot protection</h4>
            <CaptchaPanel
              scenarioId="captcha"
              onDemoDisabled={onDemoDisabled}
            />
          </div>
        );
      case "oauth":
        return (
          <div className="flex flex-col gap-2">
            {activeOauthOptions.map((option) => (
              <button
                key={option.id}
                type="button"
                onClick={() => {
                  window.location.href = loginUrl(option.id, oauthMode);
                }}
                className={`flex items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium shadow-sm transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2 ${
                  PROVIDER_STYLES[option.id] ?? PROVIDER_FALLBACK_STYLE
                }`}
              >
                Continue with {option.label}
              </button>
            ))}
            <div className="mt-1 flex items-center justify-center gap-2 text-xs text-slate-400">
              <span>Identity mode:</span>
              <div className="inline-flex rounded-md border border-slate-800 p-0.5">
                <ModeButton
                  label="Session"
                  active={oauthMode === "session"}
                  onClick={() => setOauthMode("session")}
                />
                <ModeButton
                  label="Stateless (JWT)"
                  active={oauthMode === "jwt"}
                  onClick={() => setOauthMode("jwt")}
                />
              </div>
            </div>
          </div>
        );
      case "passkeys":
        return (
          <div className="rounded-md border border-slate-800 p-4">
            <h4 className="mb-2 text-sm font-medium text-slate-200">Passkey</h4>
            <PasskeysPanel
              scenarioId="passkeys"
              onDemoDisabled={onDemoDisabled}
            />
          </div>
        );
      case "totp":
        return (
          <div className="rounded-md border border-slate-800 p-4">
            <h4 className="mb-2 text-sm font-medium text-slate-200">Authenticator app</h4>
            <TotpPanel scenarioId="totp" onDemoDisabled={onDemoDisabled} />
          </div>
        );
      case "resource":
        return (
          <div className="rounded-md border border-slate-800 p-4">
            <h4 className="mb-2 text-sm font-medium text-slate-200">Protected API route</h4>
            <ResourcePanel
              scenarioId="resource"
              onDemoDisabled={onDemoDisabled}
            />
          </div>
        );
      default:
        return null;
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-slate-100">Sign in</h2>
        <p className="text-sm text-slate-400">
          A real sign-in screen assembled from what you chose in step 1. Each panel shows
          the request it made and what came back, so nothing here has to be taken on
          trust.
        </p>
      </div>

      <div>
        <div className="rounded-lg border border-slate-800 bg-slate-900 p-6">
          {oauthReturn && (
            <OAuthReturnBanner result={oauthReturn} onDismiss={onDismissOauthReturn} />
          )}

          {visible.length === 0 ? (
            <p className="text-sm text-slate-400">
              Nothing is turned on yet.{" "}
              <button
                type="button"
                onClick={onBack}
                className="font-medium text-slate-200 underline underline-offset-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 rounded"
              >
                Go back to step 1
              </button>{" "}
              to choose something.
            </p>
          ) : (
            <div className="mx-auto flex max-w-2xl flex-col gap-5">
              <div className="text-center">
                <h3 className="text-base font-semibold text-slate-100">
                  Sign in to Authkestra
                </h3>
                <p className="text-sm text-slate-400">
                  {signInMethodOn
                    ? "Choose how you'd like to continue."
                    : "No sign-in method is on — try what you did turn on below."}
                </p>
              </div>

              {visible.map((id, i) => (
                <Fragment key={id}>
                  {i > 0 && <Divider />}
                  {renderPanel(id)}
                </Fragment>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="flex items-center justify-between">
        <button
          type="button"
          onClick={onBack}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2"
        >
          Back
        </button>
        <button
          type="button"
          onClick={onContinue}
          className="rounded-md bg-slate-200 px-4 py-2 text-sm font-medium text-slate-900 transition hover:bg-slate-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2"
        >
          Continue
        </button>
      </div>
    </div>
  );
}

function ModeButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={`rounded px-2 py-1 text-xs font-medium transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 ${
        active ? "bg-slate-200 text-slate-900" : "text-slate-400 hover:bg-slate-800"
      }`}
    >
      {label}
    </button>
  );
}

function Divider() {
  return (
    <div className="flex items-center gap-3 text-xs text-slate-400">
      <div className="h-px flex-1 bg-slate-700" />
      or
      <div className="h-px flex-1 bg-slate-700" />
    </div>
  );
}

function describeOauthErrorReason(reason: string): string {
  switch (reason) {
    case "missing_code":
      return "the provider didn't send back a code";
    case "unknown_provider":
      return "that provider isn't configured on this deployment";
    case "exchange_failed":
      return "the code exchange with the provider failed";
    case "state_missing":
      return "the browser didn't send back the flow's state cookie — this usually means more than 15 minutes passed, cookies were blocked, or the flow was started in a different browser";
    case "state_invalid":
      return "the flow's state cookie was invalid or tampered with";
    case "callback_failed":
      return "the callback from the provider failed unexpectedly";
    case "demo_disabled":
      return "OAuth is temporarily switched off";
    default:
      return reason;
  }
}

function OAuthReturnBanner({
  result,
  onDismiss,
}: {
  result: OAuthReturn;
  onDismiss: () => void;
}) {
  const styles: Record<OAuthReturn["status"], string> = {
    success: "border-emerald-500/30 bg-emerald-500/10 text-emerald-300",
    // Cancelling at the provider is an ordinary outcome — calm amber, not red.
    denied: "border-amber-500/30 bg-amber-500/10 text-amber-300",
    error: "border-red-500/30 bg-red-500/10 text-red-300",
  };

  let message: string;
  if (result.status === "success") {
    let modeDescription = "";
    if (result.mode === "session") {
      modeDescription = " A server-side session was established with its ID in a cookie.";
    } else if (result.mode === "jwt") {
      modeDescription = " A signed token (JWT) was issued with no server-side session.";
    }
    message = `Signed in with ${result.provider}.${modeDescription}`;
  } else if (result.status === "denied") {
    message = `You cancelled signing in with ${result.provider}. No harm done — try again whenever you're ready.`;
  } else {
    message = `Couldn't complete sign-in with ${result.provider}${
      result.reason ? ` — ${describeOauthErrorReason(result.reason)}` : ""
    }.`;
  }

  // The visitor left for the provider and came back. The browser drops focus
  // at the top of a freshly loaded document, so without moving it the outcome
  // of the thing they just did is somewhere below, unannounced — and for a
  // screen-reader user, the round trip appears to have done nothing.
  //
  // `role="status"` announces it; the ref focuses it, so keyboard users
  // continue from the result rather than tabbing back to it.
  const bannerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    bannerRef.current?.focus();
  }, []);

  return (
    <div
      ref={bannerRef}
      tabIndex={-1}
      role="status"
      className={`mb-4 flex items-start justify-between gap-3 rounded-md border px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-current ${styles[result.status]}`}
    >
      <p>{message}</p>
      <button
        type="button"
        onClick={onDismiss}
        className="shrink-0 rounded text-xs underline underline-offset-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-current"
      >
        Dismiss
      </button>
    </div>
  );
}
