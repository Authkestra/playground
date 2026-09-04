"use client";

import type { ConfigDiff, ControlValue, DemoConfig, ScenarioSpec } from "@playground/api-types";
import ScenarioPanel, { isControlValueActive } from "@/components/ScenarioPanel";
import DiffViewer from "@/components/DiffViewer";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  pendingIds: Set<string>;
  onChange: (id: string, value: ControlValue) => void;
  diff: ConfigDiff | null;
  diffScenarioName: string | null;
  onContinue: () => void;
}

export default function StepChooseMethods({
  scenarios,
  config,
  pendingIds,
  onChange,
  diff,
  diffScenarioName,
  onContinue,
}: Props) {
  const anyActive = scenarios.some((s) => isControlValueActive(config?.scenarios[s.id]));

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-slate-800">Choose sign-in methods</h2>
        <p className="text-sm text-slate-500">
          Pick any combination — GitHub, Google, passkeys, an authenticator app, or all of
          them. Every change below shows exactly how it reshapes the config.
        </p>
      </div>

      <ScenarioPanel
        scenarios={scenarios}
        config={config}
        pendingIds={pendingIds}
        disabled={false}
        onChange={onChange}
        showActionPanels={false}
        showTryButton={false}
      />

      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-500">
          Config diff{diffScenarioName ? ` — ${diffScenarioName}` : ""}
        </h3>
        <div className="rounded-lg border border-slate-200 bg-white p-4">
          <DiffViewer diff={diff} />
        </div>
      </section>

      <div className="flex flex-col items-start gap-2 sm:flex-row sm:items-center sm:justify-between">
        <button
          type="button"
          onClick={onContinue}
          disabled={!anyActive}
          className="rounded-md bg-slate-800 px-4 py-2 text-sm font-medium text-white transition hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-500 focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-slate-300"
        >
          Continue
        </button>
        {!anyActive && (
          <p className="text-xs text-slate-500">
            Turn on at least one sign-in method above to continue.
          </p>
        )}
      </div>
    </div>
  );
}
