"use client";

import type { ComponentType, ReactNode } from "react";
import type { ControlValue, DemoConfig, ScenarioSpec } from "@playground/api-types";
import TotpPanel from "@/components/TotpPanel";
import PasskeysPanel from "@/components/PasskeysPanel";
import { cn } from "@/lib/cn";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Switch } from "@/components/ui/switch";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";

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
  onDemoDisabled?: () => void;
  /** Show the per-scenario ceremony UI (TOTP/passkeys) inline. Defaults to true. */
  showActionPanels?: boolean;
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
  onDemoDisabled,
  showActionPanels = true,
}: Props) {
  if (scenarios.length === 0) {
    return <p className="text-sm text-muted-foreground">No scenarios published yet.</p>;
  }

  return (
    <div className="flex flex-col gap-4">
        {disabled && disabledReason && (
          <p className="text-xs text-muted-foreground">{disabledReason}</p>
        )}
        {scenarios.map((scenario) => {
          const value = config?.scenarios[scenario.id];
          const active = isControlValueActive(value);
          const unmetDeps = scenario.depends_on.filter(
            (dep) => !isSatisfied(config, dep),
          );
          const isPending = pendingIds.has(scenario.id);
          const controlDisabled =
            disabled || !scenario.available || unmetDeps.length > 0 || isPending;
          const actions = scenario.actions ?? [];
          const ActionPanel =
            showActionPanels && actions.length > 0 ? ACTION_PANELS[scenario.id] : undefined;
          const showActionPanel =
            ActionPanel &&
            !disabled &&
            scenario.available &&
            unmetDeps.length === 0 &&
            isSatisfied(config, scenario.id);

          const inlineControl = scenario.control.kind === "toggle";

          const disabledExplanation = !controlDisabled
            ? undefined
            : isPending
              ? "Applying your last change…"
              : disabled && disabledReason
                ? disabledReason
                : !scenario.available && scenario.unavailable_reason
                  ? scenario.unavailable_reason
                  : unmetDeps.length > 0
                    ? `Requires ${unmetDeps
                        .map((id) => scenarios.find((s) => s.id === id)?.name ?? id)
                        .join(", ")}`
                    : undefined;

          return (
            <Card
              key={scenario.id}
              className={cn(
                "transition-colors",
                active && !controlDisabled && "border-primary/40 bg-primary/[0.03]",
                controlDisabled && "opacity-75",
              )}
            >
              <CardHeader className="flex flex-row items-start justify-between gap-4 space-y-0">
                <div className="space-y-1.5">
                  <CardTitle className="flex items-center gap-2 text-base">
                    {scenario.name}
                    {active && (
                      <Badge variant="secondary" className="font-normal">
                        Active
                      </Badge>
                    )}
                  </CardTitle>
                  <CardDescription>{scenario.summary}</CardDescription>
                  {/*
                    Why a disabled control explains itself in visible text rather
                    than a tooltip: a tooltip on a disabled control is unreachable.
                    A `disabled` element takes neither focus nor pointer events, so
                    the trigger never fires for a keyboard user and fires only
                    inconsistently for a mouse — which is why the version this
                    replaced had to wrap the control in a bare div to catch hovers,
                    and still left the keyboard with nothing.

                    Not gated on `!available`: the two states are independent.
                    The kill switch clears `available`, but a scenario can also be
                    unusable while still "available" — OAuth with no provider
                    credentials is exactly that, and it is the live case today.
                    Gating on `available` would leave that one silently unexplained,
                    which is the dead end this field exists to prevent.
                  */}
                  {disabledExplanation && (
                    <p
                      id={`${scenario.id}-disabled-reason`}
                      className="text-xs text-warning-foreground"
                    >
                      {disabledExplanation}
                    </p>
                  )}
                </div>
                {/*
                  Only a toggle rides on the header row. A `select_one` or
                  `select_many` is a stack of three or four options — OAuth's
                  provider list, the captcha vendors — and putting that in the
                  right-hand column squeezes it into a narrow strip beside the
                  description while the header grows to match its height. Those
                  belong in the body, at full width.
                */}
                {inlineControl && (
                  <div className="shrink-0 pt-0.5">
                    <ScenarioControl
                      scenario={scenario}
                      value={value}
                      disabled={controlDisabled}
                      describedBy={
                        disabledExplanation ? `${scenario.id}-disabled-reason` : undefined
                      }
                      onChange={(next) => onChange(scenario.id, next)}
                    />
                  </div>
                )}
              </CardHeader>

              {!inlineControl && (
                <CardContent className="pt-0">
                  <ScenarioControl
                    scenario={scenario}
                    value={value}
                    disabled={controlDisabled}
                    describedBy={
                      disabledExplanation ? `${scenario.id}-disabled-reason` : undefined
                    }
                    onChange={(next) => onChange(scenario.id, next)}
                  />
                </CardContent>
              )}

              {showActionPanel && ActionPanel && (
                <CardContent className="pt-0">
                  <Separator className="mb-4" />
                  <ActionPanel
                    scenarioId={scenario.id}
                    onDemoDisabled={onDemoDisabled ?? (() => {})}
                  />
                </CardContent>
              )}
            </Card>
          );
        })}
    </div>
  );
}

function ScenarioControl({
  scenario,
  value,
  disabled,
  describedBy,
  onChange,
}: {
  scenario: ScenarioSpec;
  value: ControlValue | undefined;
  disabled: boolean;
  /** Id of the visible text saying why this is disabled, when it is. */
  describedBy?: string;
  onChange: (value: ControlValue) => void;
}) {
  const control = scenario.control;

  if (control.kind === "toggle") {
    const enabled = value?.kind === "toggle" ? value.enabled : false;
    return (
      <Switch
        checked={enabled}
        onCheckedChange={(checked) => onChange({ kind: "toggle", enabled: checked })}
        disabled={disabled}
        aria-label={scenario.name}
        aria-describedby={describedBy}
      />
    );
  }

  if (control.kind === "select_one") {
    const selected = value?.kind === "select_one" ? value.selected : null;
    if (control.options.length === 0) {
      return <EmptyControlNote scenario={scenario} />;
    }
    return (
      <RadioGroup
        value={selected ?? undefined}
        onValueChange={(next) => onChange({ kind: "select_one", selected: next })}
        disabled={disabled}
        aria-label={scenario.name}
        aria-describedby={describedBy}
        className="gap-2"
      >
        {control.options.map((option) => {
          const id = `${scenario.id}-${option.id}`;
          return (
            <div key={option.id} className="flex items-center gap-2">
              <RadioGroupItem value={option.id} id={id} />
              <Label htmlFor={id} className="font-normal text-foreground">
                {option.label}
              </Label>
            </div>
          );
        })}
      </RadioGroup>
    );
  }

  // select_many
  const selected = value?.kind === "select_many" ? value.selected : [];
  if (control.options.length === 0) {
    return <EmptyControlNote scenario={scenario} />;
  }
  return (
    <div
      role="group"
      aria-label={scenario.name}
      aria-describedby={describedBy}
      className="flex flex-col gap-2"
    >
      {control.options.map((option) => {
        const checked = selected.includes(option.id);
        const id = `${scenario.id}-${option.id}`;
        return (
          <div key={option.id} className="flex items-center gap-2">
            <Checkbox
              id={id}
              checked={checked}
              disabled={disabled}
              onCheckedChange={(next) => {
                const nextSelected =
                  next === true
                    ? [...selected, option.id]
                    : selected.filter((optionId) => optionId !== option.id);
                onChange({ kind: "select_many", selected: nextSelected });
              }}
            />
            <Label htmlFor={id} className="font-normal text-foreground">
              {option.label}
            </Label>
          </div>
        );
      })}
    </div>
  );
}

/**
 * Stands in for a control that has nothing to offer — an OAuth picker on a
 * deployment with no provider credentials, say. Without it the card renders a
 * heading and summary above an empty box, which reads as broken rather than as
 * a property of this deployment.
 *
 * The `unavailable_reason` itself is already shown in amber above the control,
 * so this only accounts for the empty space rather than repeating it.
 */
function EmptyControlNote({ scenario }: { scenario: ScenarioSpec }) {
  return (
    <p className="text-xs text-muted-foreground">
      {scenario.unavailable_reason
        ? "Nothing to choose from here."
        : "Nothing to choose from on this deployment."}
    </p>
  );
}
