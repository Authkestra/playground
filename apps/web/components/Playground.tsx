"use client";

import { useCallback, useEffect, useState } from "react";
import type {
  ConfigDiff,
  ControlValue,
  DemoConfig,
  DemoSessionView,
  HealthResponse,
  ScenarioSpec,
} from "@playground/api-types";
import {
  API_BASE,
  configureScenario,
  getHealth,
  getScenarios,
  getSession,
  resetSession,
} from "@/lib/api";
import { clearOAuthReturnParams, readOAuthReturn, type OAuthReturn } from "@/lib/oauth";
import SessionBar from "@/components/SessionBar";
import ScenarioPanel from "@/components/ScenarioPanel";
import StepIndicator from "@/components/StepIndicator";
import StepChooseMethods from "@/components/StepChooseMethods";
import StepSignIn from "@/components/StepSignIn";
import StepDownload from "@/components/StepDownload";

type Phase = "loading" | "unavailable" | "explainer" | "ready";
type Step = 1 | 2 | 3;

export default function Playground() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [, setHealth] = useState<HealthResponse | null>(null);
  const [session, setSession] = useState<DemoSessionView | null>(null);
  const [scenarios, setScenarios] = useState<ScenarioSpec[]>([]);
  const [config, setConfig] = useState<DemoConfig | null>(null);
  const [diff, setDiff] = useState<ConfigDiff | null>(null);
  const [diffScenarioName, setDiffScenarioName] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const [resetting, setResetting] = useState(false);

  const [step, setStep] = useState<Step>(1);
  const [maxReached, setMaxReached] = useState<Step>(1);
  const [oauthReturn, setOauthReturn] = useState<OAuthReturn | null>(null);

  const goToStep = useCallback((next: Step) => {
    setStep(next);
    setMaxReached((prev) => (next > prev ? next : prev));
  }, []);

  const load = useCallback(async () => {
    setPhase("loading");
    setBanner(null);

    const healthResult = await getHealth();
    if (!healthResult.ok) {
      setPhase("unavailable");
      return;
    }
    setHealth(healthResult.data);

    // Best-effort: fetch the scenario specs even in explainer mode so the
    // disabled controls still render (rather than an empty page).
    const scenariosResult = await getScenarios();
    if (scenariosResult.ok) {
      setScenarios(scenariosResult.data);
    }

    if (!healthResult.data.demo_enabled) {
      setPhase("explainer");
      return;
    }

    if (!scenariosResult.ok) {
      switch (scenariosResult.error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          break;
        case "unavailable":
          setPhase("unavailable");
          break;
        case "rate_limited":
          setBanner(scenariosResult.error.detail);
          setPhase("unavailable");
          break;
        default:
          setBanner(`Could not load scenarios (${scenariosResult.error.detail}).`);
          setPhase("unavailable");
      }
      return;
    }

    const sessionResult = await getSession();
    if (!sessionResult.ok) {
      switch (sessionResult.error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          break;
        case "unavailable":
          setPhase("unavailable");
          break;
        case "rate_limited":
          setBanner(sessionResult.error.detail);
          setPhase("unavailable");
          break;
        default:
          setBanner(`Could not load session (${sessionResult.error.detail}).`);
          setPhase("unavailable");
      }
      return;
    }

    setSession(sessionResult.data);
    setConfig(sessionResult.data.config);
    setPhase("ready");

    // The browser may just have navigated back from an OAuth provider — the
    // outcome arrives as query params on this very load. Read them, land the
    // visitor on step 2 to see it, then scrub the URL so a reload or a
    // shared link doesn't replay the same result.
    const parsedOauthReturn = readOAuthReturn(window.location.search);
    if (parsedOauthReturn) {
      setOauthReturn(parsedOauthReturn);
      clearOAuthReturnParams();
      goToStep(2);
    }
  }, [goToStep]);

  useEffect(() => {
    void load();
  }, [load]);

  const handleReset = useCallback(async () => {
    setResetting(true);
    setBanner(null);
    const result = await resetSession();
    setResetting(false);

    if (!result.ok) {
      switch (result.error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          break;
        case "unavailable":
          setPhase("unavailable");
          break;
        case "rate_limited":
          setBanner(result.error.detail);
          break;
        default:
          setBanner("Could not reset the session. Please try again.");
      }
      return;
    }

    setSession(result.data);
    setConfig(result.data.config);
    setDiff(null);
    setDiffScenarioName(null);
    setOauthReturn(null);
    setStep(1);
    setMaxReached(1);
  }, []);

  const handleChange = useCallback(
    async (id: string, value: ControlValue) => {
      setPendingIds((prev) => new Set(prev).add(id));
      setBanner(null);

      // Move the control immediately and roll back if the server disagrees.
      // Waiting for the round trip made the switch feel dead — and on a
      // free-tier host that has spun down, the first interaction can take
      // tens of seconds, which reads as nothing happening at all.
      const previousConfig = config;
      setConfig((prev) =>
        prev
          ? { ...prev, scenarios: { ...prev.scenarios, [id]: value } }
          : prev,
      );

      const result = await configureScenario(id, { value });

      setPendingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });

      if (!result.ok) {
        // Put the control back where it was, so the UI never claims a change
        // the server did not accept.
        setConfig(previousConfig);
        switch (result.error.kind) {
          case "demo_disabled":
            setPhase("explainer");
            break;
          case "unavailable":
            setPhase("unavailable");
            break;
          case "rate_limited":
            setBanner(result.error.detail);
            break;
          default:
            setBanner(`Could not update "${id}": ${result.error.detail}`);
        }
        return;
      }

      // The server's copy is authoritative — it may normalise the value.
      setConfig(result.data.config);
      setDiff(result.data.diff);
      setDiffScenarioName(scenarios.find((s) => s.id === id)?.name ?? id);
    },
    [scenarios, config],
  );

  if (phase === "loading") {
    return (
      <main className="mx-auto flex max-w-3xl flex-col gap-6 p-8" aria-busy="true" aria-label="Loading playground">
        <Header />
        <div className="flex flex-col gap-3">
          <div className="h-10 bg-slate-800/60 rounded animate-pulse" />
          <div className="h-6 bg-slate-800/60 rounded animate-pulse w-3/4" />
        </div>
        <div className="flex flex-col gap-2">
          <div className="h-5 bg-slate-800/60 rounded animate-pulse w-1/2" />
          <div className="h-5 bg-slate-800/60 rounded animate-pulse w-2/3" />
        </div>
        <p className="text-sm text-slate-500">
          Loading playground… The API runs on a free tier and can take up to a minute to wake
          up on its first request.
        </p>
      </main>
    );
  }

  if (phase === "unavailable") {
    return (
      <main className="mx-auto flex max-w-3xl flex-col gap-6 p-8">
        <Header />
        <div className="rounded-lg border border-slate-800 bg-slate-900 p-6 text-center">
          <h2 className="font-medium text-slate-200">API unavailable</h2>
          <p className="mt-2 text-sm text-slate-400">
            The playground couldn&apos;t reach the API at{" "}
            <code className="font-mono">{API_BASE}</code>. Start the backend and
            reload this page.
          </p>
          {banner && <p className="mt-2 text-sm text-amber-400">{banner}</p>}
          <button
            type="button"
            onClick={() => void load()}
            className="mt-4 rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800"
          >
            Retry
          </button>
        </div>
      </main>
    );
  }

  if (phase === "explainer") {
    return (
      <main className="mx-auto flex max-w-3xl flex-col gap-6 p-8">
        <Header />
        <div className="rounded-lg border border-slate-800 bg-slate-900 p-6">
          <h2 className="font-medium text-slate-200">Demo currently disabled</h2>
          <p className="mt-2 text-sm text-slate-400">
            The live playground is switched off right now, so controls below are
            shown for reference only. This is expected behaviour, not an error —
            check back later to try the flows interactively.
          </p>
        </div>
        <section className="flex flex-col gap-3">
          <h2 className="text-sm font-semibold uppercase tracking-wide text-slate-400">
            Scenarios
          </h2>
          <ScenarioPanel
            scenarios={scenarios}
            config={config}
            pendingIds={pendingIds}
            disabled
            disabledReason="Controls are disabled while the demo is switched off."
            onChange={() => {}}
          />
        </section>
      </main>
    );
  }

  return (
    <main className="mx-auto flex max-w-5xl flex-col gap-6 p-8">
      <Header />
      <div
        aria-live="polite"
        className="min-h-10"
      >
        {banner && (
          <div className="rounded-md border border-amber-500/30 bg-amber-500/10 px-4 py-2 text-sm text-amber-300">
            {banner}
          </div>
        )}
      </div>
      <SessionBar
        session={session}
        onReset={() => void handleReset()}
        resetting={resetting}
      />

      <StepIndicator current={step} maxReached={maxReached} onNavigate={goToStep} />

      {step === 1 && (
        <StepChooseMethods
          scenarios={scenarios}
          config={config}
          pendingIds={pendingIds}
          onChange={(id, value) => void handleChange(id, value)}
          diff={diff}
          diffScenarioName={diffScenarioName}
          onContinue={() => goToStep(2)}
        />
      )}

      {step === 2 && (
        <StepSignIn
          scenarios={scenarios}
          config={config}
          oauthReturn={oauthReturn}
          onDismissOauthReturn={() => setOauthReturn(null)}
          onDemoDisabled={() => setPhase("explainer")}
          onBack={() => goToStep(1)}
          onContinue={() => goToStep(3)}
        />
      )}

      {step === 3 && (
        <StepDownload
          scenarios={scenarios}
          config={config}
          onDemoDisabled={() => setPhase("explainer")}
          onBack={() => goToStep(2)}
        />
      )}
    </main>
  );
}

/** The framework's own site, which links back here. */
const AUTHKESTRA_SITE = "https://authkestra.com";

function Header() {
  return (
    <header className="flex flex-wrap items-start justify-between gap-3">
      <div>
        <h1 className="text-2xl font-semibold text-slate-100">Authkestra Playground</h1>
        <p className="text-sm text-slate-400">
          Choose your sign-in methods, see the config diff, and try the flows live.
        </p>
      </div>
      {/*
        A visitor who likes what they see should not have to go hunting for the
        framework — the playground exists to send people there.
      */}
      <a
        href={AUTHKESTRA_SITE}
        target="_blank"
        rel="noreferrer"
        className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2 focus-visible:ring-offset-slate-950"
      >
        authkestra docs
        <span aria-hidden="true">→</span>
        <span className="sr-only">(opens in a new tab)</span>
      </a>
    </header>
  );
}
