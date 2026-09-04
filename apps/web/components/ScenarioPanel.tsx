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
  onTry?: (id: string) => void;
  tryingIds?: Set<string>;
  tryResults?: Record<string, TryResultState>;
  onDemoDisabled?: () => void;
  /** Show the per-scenario ceremony UI (TOTP/passkeys) inline. Defaults to true. */
  showActionPanels?: boolean;
  /** Show the debug "Try it" button/result. Defaults to true. */
  showTryButton?: boolean;
}

/** Whether a control's current value counts as "the visitor turned this on". */
export function isControlValueActive(value: ControlValue | undefined): boolean {
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

function isSatisfied(config: DemoConfig | null, dependencyId: string): boolean {
  return isControlValueActive(config?.scenarios[dependencyId]);
}

export default function ScenarioPanel({
  scenarios,
  config,
  pendingIds,
  disabled,
  disabledReason,
  onChange,
  onTry,
  tryingIds = new Set(),
  tryResults = {},
  onDemoDisabled,
  showActionPanels = true,
  showTryButton = true,
}: Props) {
  if (scenarios.length === 0) {
    return <p className="text-sm text-slate-500">No scenarios published yet.</p>;
  }

  return (
    <div className="flex flex-col gap-4">
      {disabled && disabledReason && (
        <p className="text-xs text-slate-400">{disabledReason}</p>
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
        const ActionPanel =
          showActionPanels && actions.length > 0 ? ACTION_PANELS[scenario.id] : undefined;
        const showActionPanel =
          ActionPanel &&
          !disabled &&
          scenario.available &&
          unmetDeps.length === 0 &&
          isSatisfied(config, scenario.id);

        return (
          <div
            key={scenario.id}
            className="rounded-lg border border-slate-800 bg-slate-900 p-4"
          >
            <div className="flex items-start justify-between gap-4">
              <div>
                <h3 className="font-medium text-slate-100">{scenario.name}</h3>
                <p className="text-sm text-slate-400">{scenario.summary}</p>
                {!scenario.available && (
                  <p className="mt-1 text-xs text-amber-400">
                    Disabled by kill switch.
                  </p>
                )}
                {scenario.available && unmetDeps.length > 0 && (
                  <p className="mt-1 text-xs text-amber-400">
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

            {showTryButton && (
              <div className="mt-3 flex items-center gap-3">
                <button
                  type="button"
                  onClick={() => onTry?.(scenario.id)}
                  disabled={disabled || !scenario.available || tryingIds.has(scenario.id)}
                  className="rounded-md border border-slate-700 px-2.5 py-1 text-xs font-medium text-slate-300 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {tryingIds.has(scenario.id) ? "Trying…" : "Try it"}
                </button>
                {tryResult && (
                  <span
                    className={`text-xs ${
                      tryResult.outcome === "ok" ? "text-emerald-400" : "text-amber-400"
                    }`}
                  >
                    {tryResult.detail}
                  </span>
                )}
              </div>
            )}

            {showActionPanel && ActionPanel && (
              <div className="mt-4 border-t border-slate-800 pt-4">
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
        className={`relative h-6 w-11 shrink-0 rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 ${
          enabled ? "bg-slate-200" : "bg-slate-700"
        }`}
      >
        {/*
          Track is 44x24 and the knob 20, so the knob needs a 2px inset on both
          sides: 2px off, and 44-20-2 = 22px on. `translate-x-5` (20px) left a
          4px gap on the right against 2px on the left, which read as a
          slightly crooked switch. The ring gives the white knob definition
          against the light off-state track.
        */}
        <span
          className={`absolute top-0.5 h-5 w-5 rounded-full bg-slate-900 shadow-sm ring-1 ring-black/5 transition-transform duration-200 ease-out ${
            enabled ? "translate-x-[22px]" : "translate-x-0.5"
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
            className="flex items-center gap-2 text-sm text-slate-300"
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
            className="flex items-center gap-2 text-sm text-slate-300"
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
