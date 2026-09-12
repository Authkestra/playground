"use client";

import type { ConfigDiff, ControlValue, DemoConfig, ScenarioSpec } from "@playground/api-types";
import ScenarioPanel, { isControlValueActive } from "@/components/ScenarioPanel";
import DiffViewer from "@/components/DiffViewer";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

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
        <h2 className="text-lg font-semibold text-foreground">Choose sign-in methods</h2>
        <p className="text-sm text-muted-foreground">
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
      />

      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
          Config diff{diffScenarioName ? ` — ${diffScenarioName}` : ""}
        </h3>
        <Card>
          <CardContent className="p-4">
            <DiffViewer diff={diff} />
          </CardContent>
        </Card>
      </section>

      <div className="flex flex-col items-start gap-2 sm:flex-row sm:items-center sm:justify-between">
        <Button onClick={onContinue} disabled={!anyActive}>
          Continue
        </Button>
        {!anyActive && (
          <p className="text-xs text-muted-foreground">
            Turn on at least one sign-in method above to continue.
          </p>
        )}
      </div>
    </div>
  );
}
