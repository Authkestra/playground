"use client";

import type { ComponentType } from "react";
import type { ControlValue, DemoConfig, ScenarioSpec } from "@playground/api-types";
import TotpPanel from "@/components/TotpPanel";
import PasskeysPanel from "@/components/PasskeysPanel";

export interface TryResultState {
  outcome: string;
  detail: string;
}

interface ActionPanelProps {
  scenarioId: string;
  onDemoDisabled: () => void;
}

// Ceremony UI for scenarios that expose `actions` (e.g. multi-step flows
// beyond the generic toggle/select controls), keyed by scenario id. A
// scenario with no entry here still renders normally — it just doesn't get
// an extra panel.
const ACTION_PANELS: Record<string, ComponentType<ActionPanelProps>> = {
  totp: TotpPanel,
  passkeys: PasskeysPanel,
};

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  pendingIds: Set<string>;
  disabled: boolean;
  disabledReason?: string;
  onChange: (id: string, value: ControlValue) => void;
  onTry: (id: string) => void;
  tryingIds: Set<string>;
  tryResults: Record<string, TryResultState>;
  onDemoDisabled?: () => void;
}

function isSatisfied(config: DemoConfig | null, dependencyId: string): boolean {
  const value = config?.scenarios[dependencyId];
  if (!value) return false;
  switch (value.kind) {
    case "toggle":
      return value.enabled;
    case "select_one":
      return value.selected !== null;
    case "select_many":
      return value.selected.length > 0;
    default:
      return false;
  }
}

export default function ScenarioPanel({
  scenarios,
  config,
  pendingIds,
  disabled,
  disabledReason,
  onChange,
  onTry,
  tryingIds,
  tryResults,
  onDemoDisabled,
}: Props) {
  if (scenarios.length === 0) {
    return <p className="text-sm text-slate-400">No scenarios published yet.</p>;
  }

  return (
    <div className="flex flex-col gap-4">
      {disabled && disabledReason && (
        <p className="text-xs text-slate-500">{disabledReason}</p>
      )}
      {scenarios.map((scenario) => {
        const value = config?.scenarios[scenario.id];
        const unmetDeps = scenario.depends_on.filter(
          (dep) => !isSatisfied(config, dep),
        );
        const isPending = pendingIds.has(scenario.id);
        const controlDisabled =
          disabled || !scenario.available || unmetDeps.length > 0 || isPending;
        const tryResult = tryResults[scenario.id];
        const actions = scenario.actions ?? [];
        const ActionPanel = actions.length > 0 ? ACTION_PANELS[scenario.id] : undefined;
        const showActionPanel =
          ActionPanel &&
          !disabled &&
          scenario.available &&
          unmetDeps.length === 0 &&
          isSatisfied(config, scenario.id);

        return (
          <div
            key={scenario.id}
            className="rounded-lg border border-slate-200 bg-white p-4"
          >
            <div className="flex items-start justify-between gap-4">
              <div>
                <h3 className="font-medium text-slate-800">{scenario.name}</h3>
                <p className="text-sm text-slate-500">{scenario.summary}</p>
                {!scenario.available && (
                  <p className="mt-1 text-xs text-amber-600">
                    Disabled by kill switch.
                  </p>
                )}
                {scenario.available && unmetDeps.length > 0 && (
                  <p className="mt-1 text-xs text-amber-600">
                    Requires: {unmetDeps.join(", ")}
                  </p>
                )}
              </div>
              <div className="shrink-0">
                <ScenarioControl
                  scenario={scenario}
                  value={value}
                  disabled={controlDisabled}
                  onChange={(next) => onChange(scenario.id, next)}
                />
              </div>
            </div>

            <div className="mt-3 flex items-center gap-3">
              <button
                type="button"
                onClick={() => onTry(scenario.id)}
                disabled={disabled || !scenario.available || tryingIds.has(scenario.id)}
                className="rounded-md border border-slate-300 px-2.5 py-1 text-xs font-medium text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
              >
                {tryingIds.has(scenario.id) ? "Trying…" : "Try it"}
              </button>
              {tryResult && (
                <span
                  className={`text-xs ${
                    tryResult.outcome === "ok" ? "text-emerald-600" : "text-amber-600"
                  }`}
                >
                  {tryResult.detail}
                </span>
              )}
            </div>

            {showActionPanel && ActionPanel && (
              <div className="mt-4 border-t border-slate-100 pt-4">
                <ActionPanel
                  scenarioId={scenario.id}
                  onDemoDisabled={onDemoDisabled ?? (() => {})}
                />
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function ScenarioControl({
  scenario,
  value,
  disabled,
  onChange,
}: {
  scenario: ScenarioSpec;
  value: ControlValue | undefined;
  disabled: boolean;
  onChange: (value: ControlValue) => void;
}) {
  const control = scenario.control;

  if (control.kind === "toggle") {
    const enabled = value?.kind === "toggle" ? value.enabled : false;
    return (
      <button
        type="button"
        role="switch"
        aria-checked={enabled}
        aria-label={scenario.name}
        disabled={disabled}
        onClick={() => onChange({ kind: "toggle", enabled: !enabled })}
        className={`relative h-6 w-11 rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
          enabled ? "bg-slate-800" : "bg-slate-300"
        }`}
      >
        <span
          className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
            enabled ? "translate-x-5" : "translate-x-0.5"
          }`}
        />
      </button>
    );
  }

  if (control.kind === "select_one") {
    const selected = value?.kind === "select_one" ? value.selected : null;
    return (
      <div className="flex flex-col gap-1">
        {control.options.map((option) => (
          <label
            key={option.id}
            className="flex items-center gap-2 text-sm text-slate-600"
          >
            <input
              type="radio"
              name={`${scenario.id}-select-one`}
              checked={selected === option.id}
              disabled={disabled}
              onChange={() => onChange({ kind: "select_one", selected: option.id })}
            />
            {option.label}
          </label>
        ))}
      </div>
    );
  }

  // select_many
  const selected = value?.kind === "select_many" ? value.selected : [];
  return (
    <div className="flex flex-col gap-1">
      {control.options.map((option) => {
        const checked = selected.includes(option.id);
        return (
          <label
            key={option.id}
            className="flex items-center gap-2 text-sm text-slate-600"
          >
            <input
              type="checkbox"
              checked={checked}
              disabled={disabled}
              onChange={() => {
                const next = checked
                  ? selected.filter((id) => id !== option.id)
                  : [...selected, option.id];
                onChange({ kind: "select_many", selected: next });
              }}
            />
            {option.label}
          </label>
        );
      })}
    </div>
  );
}
