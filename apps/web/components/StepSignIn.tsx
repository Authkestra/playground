"use client";

import { useCallback, useEffect, useState } from "react";
import type { DemoConfig, FlowEvent, OAuthMode, ScenarioSpec } from "@playground/api-types";
import { getSessionEvents } from "@/lib/api";
import { loginUrl, type OAuthReturn } from "@/lib/oauth";
import { isControlValueActive } from "@/components/ScenarioPanel";
import TotpPanel from "@/components/TotpPanel";
import PasskeysPanel from "@/components/PasskeysPanel";
import FlowLog from "@/components/FlowLog";

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

export default function StepSignIn({
  scenarios,
  config,
  oauthReturn,
  onDismissOauthReturn,
  onDemoDisabled,
  onBack,
  onContinue,
}: Props) {
  const [events, setEvents] = useState<FlowEvent[]>([]);
  const [eventsLoading, setEventsLoading] = useState(true);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const [oauthMode, setOauthMode] = useState<OAuthMode>("session");

  const fetchEvents = useCallback(async () => {
    const result = await getSessionEvents();
    if (!result.ok) {
      switch (result.error.kind) {
        case "demo_disabled":
          onDemoDisabled();
          return;
        case "state_unavailable":
          setEventsError(result.error.detail);
          return;
        case "unavailable":
          setEventsError(
            "Can't reach the API right now. The flow log will pick back up once it responds.",
          );
          return;
        case "rate_limited":
          setEventsError(result.error.detail);
          return;
        default:
          setEventsError(`Could not load the flow log (${result.error.detail}).`);
          return;
      }
    }
    setEventsError(null);
    setEvents(result.data);
  }, [onDemoDisabled]);

  // Poll every 5 seconds, but skip fetches while the tab is hidden. When the
  // tab becomes visible, fetch immediately to catch up on events. onAction
  // already refetches after every ceremony step, so this is a safety net for
  // events the client didn't initiate (e.g. the OAuth round trip).
  useEffect(() => {
    let cancelled = false;
    setEventsLoading(true);
    void fetchEvents().finally(() => {
      if (!cancelled) setEventsLoading(false);
    });

    const interval = setInterval(() => {
      if (!document.hidden) {
        void fetchEvents();
      }
    }, 5000);

    const handleVisibilityChange = () => {
      if (!document.hidden) {
        void fetchEvents();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      cancelled = true;
      clearInterval(interval);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [fetchEvents]);

  const oauthScenario = scenarios.find((s) => s.id === "oauth");
  const oauthValue = config?.scenarios.oauth;
  const selectedProviders = oauthValue?.kind === "select_many" ? oauthValue.selected : [];
  const oauthOptions =
    oauthScenario?.control.kind === "select_many" ? oauthScenario.control.options : [];
  const activeOauthOptions = oauthOptions.filter((o) => selectedProviders.includes(o.id));
  const oauthAvailable = oauthScenario?.available !== false && activeOauthOptions.length > 0;

  const passkeysActive = isControlValueActive(config?.scenarios.passkeys);
  const totpActive = isControlValueActive(config?.scenarios.totp);

  const hasAnyMethod = oauthAvailable || passkeysActive || totpActive;

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-slate-100">Sign in</h2>
        <p className="text-sm text-slate-400">
          A real sign-in screen assembled from what you chose in step 1, and a live log of
          what the engine is doing on the right.
        </p>
      </div>

      <div className="grid gap-6 lg:grid-cols-2 lg:items-start">
        <div className="rounded-lg border border-slate-800 bg-slate-900 p-6">
          {oauthReturn && (
            <OAuthReturnBanner result={oauthReturn} onDismiss={onDismissOauthReturn} />
          )}

          {!hasAnyMethod ? (
            <p className="text-sm text-slate-400">
              No sign-in method is turned on yet.{" "}
              <button
                type="button"
                onClick={onBack}
                className="font-medium text-slate-200 underline underline-offset-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 rounded"
              >
                Go back to step 1
              </button>{" "}
              to choose one.
            </p>
          ) : (
            <div className="mx-auto flex max-w-sm flex-col gap-5">
              <div className="text-center">
                <h3 className="text-base font-semibold text-slate-100">
                  Sign in to Authkestra
                </h3>
                <p className="text-sm text-slate-400">Choose how you&apos;d like to continue.</p>
              </div>

              {oauthAvailable && (
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
                  <div className="mt-1 flex items-center justify-center gap-2 text-xs text-slate-500">
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
              )}

              {oauthAvailable && (passkeysActive || totpActive) && <Divider />}

              {passkeysActive && (
                <div className="rounded-md border border-slate-800 p-4">
                  <h4 className="mb-2 text-sm font-medium text-slate-200">Passkey</h4>
                  <PasskeysPanel
                    scenarioId="passkeys"
                    onDemoDisabled={onDemoDisabled}
                    onAction={fetchEvents}
                  />
                </div>
              )}

              {passkeysActive && totpActive && <Divider />}

              {totpActive && (
                <div className="rounded-md border border-slate-800 p-4">
                  <h4 className="mb-2 text-sm font-medium text-slate-200">Authenticator app</h4>
                  <TotpPanel
                    scenarioId="totp"
                    onDemoDisabled={onDemoDisabled}
                    onAction={fetchEvents}
                  />
                </div>
              )}
            </div>
          )}
        </div>

        <div className="lg:sticky lg:top-6">
          <FlowLog events={events} loading={eventsLoading} error={eventsError} />
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
    <div className="flex items-center gap-3 text-xs text-slate-500">
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

  return (
    <div
      className={`mb-4 flex items-start justify-between gap-3 rounded-md border px-3 py-2 text-sm ${styles[result.status]}`}
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
