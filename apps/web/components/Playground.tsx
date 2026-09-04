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
  tryScenario,
} from "@/lib/api";
import SessionBar from "@/components/SessionBar";
import ScenarioPanel, { type TryResultState } from "@/components/ScenarioPanel";
import DiffViewer from "@/components/DiffViewer";

type Phase = "loading" | "unavailable" | "explainer" | "ready";

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
  const [tryingIds, setTryingIds] = useState<Set<string>>(new Set());
  const [tryResults, setTryResults] = useState<Record<string, TryResultState>>({});
  const [resetting, setResetting] = useState(false);

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
  }, []);

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
    setTryResults({});
  }, []);

  const handleChange = useCallback(
    async (id: string, value: ControlValue) => {
      setPendingIds((prev) => new Set(prev).add(id));
      setBanner(null);

      const result = await configureScenario(id, { value });

      setPendingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });

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
            setBanner(`Could not update "${id}": ${result.error.detail}`);
        }
        return;
      }

      setConfig(result.data.config);
      setDiff(result.data.diff);
      setDiffScenarioName(
        scenarios.find((s) => s.id === id)?.name ?? id,
      );
    },
    [scenarios],
  );

  const handleTry = useCallback(async (id: string) => {
    setTryingIds((prev) => new Set(prev).add(id));
    setBanner(null);

    const result = await tryScenario(id, {});

    setTryingIds((prev) => {
      const next = new Set(prev);
      next.delete(id);
      return next;
    });

    if (!result.ok) {
      const error = result.error;
      switch (error.kind) {
        case "demo_disabled":
          setPhase("explainer");
          return;
        case "unavailable":
          setBanner("The API became unavailable while trying this scenario.");
          return;
        case "rate_limited":
          setBanner(error.detail);
          return;
        default: {
          const message = `Could not try this scenario: ${error.detail}`;
          setTryResults((prev) => ({
            ...prev,
            [id]: { outcome: "error", detail: message },
          }));
          return;
        }
      }
    }

    setTryResults((prev) => ({ ...prev, [id]: result.data }));
  }, []);

  if (phase === "loading") {
    return (
      <main className="mx-auto flex max-w-3xl flex-col gap-6 p-8">
        <Header />
        <p className="text-sm text-slate-400">Loading playground…</p>
      </main>
    );
  }

  if (phase === "unavailable") {
    return (
      <main className="mx-auto flex max-w-3xl flex-col gap-6 p-8">
        <Header />
        <div className="rounded-lg border border-slate-200 bg-white p-6 text-center">
          <h2 className="font-medium text-slate-700">API unavailable</h2>
          <p className="mt-2 text-sm text-slate-500">
            The playground couldn&apos;t reach the API at{" "}
            <code className="font-mono">{API_BASE}</code>. Start the backend and
            reload this page.
          </p>
          {banner && <p className="mt-2 text-sm text-amber-600">{banner}</p>}
          <button
            type="button"
            onClick={() => void load()}
            className="mt-4 rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 transition hover:bg-slate-100"
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
        <div className="rounded-lg border border-slate-200 bg-white p-6">
          <h2 className="font-medium text-slate-700">Demo currently disabled</h2>
          <p className="mt-2 text-sm text-slate-500">
            The live playground is switched off right now, so controls below are
            shown for reference only. This is expected behaviour, not an error —
            check back later to try the flows interactively.
          </p>
        </div>
        <section className="flex flex-col gap-3">
          <h2 className="text-sm font-semibold uppercase tracking-wide text-slate-500">
            Scenarios
          </h2>
          <ScenarioPanel
            scenarios={scenarios}
            config={config}
            pendingIds={pendingIds}
            disabled
            disabledReason="Controls are disabled while the demo is switched off."
            onChange={() => {}}
            onTry={() => {}}
            tryingIds={tryingIds}
            tryResults={tryResults}
          />
        </section>
      </main>
    );
  }

  return (
    <main className="mx-auto flex max-w-3xl flex-col gap-6 p-8">
      <Header />
      {banner && (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-4 py-2 text-sm text-amber-700">
          {banner}
        </div>
      )}
      <SessionBar
        session={session}
        onReset={() => void handleReset()}
        resetting={resetting}
      />
      <section className="flex flex-col gap-3">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate-500">
          Scenarios
        </h2>
        <ScenarioPanel
          scenarios={scenarios}
          config={config}
          pendingIds={pendingIds}
          disabled={false}
          onChange={(id, value) => void handleChange(id, value)}
          onTry={(id) => void handleTry(id)}
          tryingIds={tryingIds}
          tryResults={tryResults}
          onDemoDisabled={() => setPhase("explainer")}
        />
      </section>
      <section className="flex flex-col gap-3">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-slate-500">
          Config diff{diffScenarioName ? ` — ${diffScenarioName}` : ""}
        </h2>
        <div className="rounded-lg border border-slate-200 bg-white p-4">
          <DiffViewer diff={diff} />
        </div>
      </section>
    </main>
  );
}

function Header() {
  return (
    <header>
      <h1 className="text-2xl font-semibold text-slate-800">Authkestra Playground</h1>
      <p className="text-sm text-slate-500">
        Toggle auth features, see the config diff, and try the flows live.
      </p>
    </header>
  );
}
