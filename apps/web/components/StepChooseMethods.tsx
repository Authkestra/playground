"use client";

import type { ConfigDiff, ControlValue, DemoConfig, ScenarioSpec } from "@playground/api-types";
import ScenarioPanel, { isControlValueActive } from "@/components/ScenarioPanel";
import DiffViewer from "@/components/DiffViewer";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ChevronRight } from "lucide-react";

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
  const changeCount = diff?.entries.length ?? 0;

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

      {/*
        A native `<details>`, not a `useState` toggle. It costs no JavaScript,
        it keeps `DiffViewer` a server component, and the browser gives the
        keyboard and screen-reader behaviour for free — a disclosure widget is
        one of the few things HTML already does correctly.

        Closed to start. The diff is the evidence behind the choices above, and
        evidence is what you consult when you want it: open by default, it
        pushed the Continue button below the fold on a laptop and made every
        toggle reflow a block most visitors never read. The summary still
        carries the count, so the fact that something changed is visible
        without expanding anything.
      */}
      <details className="group rounded-xl border bg-card text-card-foreground shadow">
          <summary className="flex cursor-pointer list-none items-center gap-2 p-4 text-sm font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
            <ChevronRight
              aria-hidden
              className="size-4 shrink-0 transition-transform duration-200 group-open:rotate-90"
            />
            Config diff{diffScenarioName ? ` — ${diffScenarioName}` : ""}
            {changeCount > 0 && (
              <Badge variant="secondary" className="ml-1 font-normal normal-case">
                {changeCount} {changeCount === 1 ? "change" : "changes"}
              </Badge>
            )}
          </summary>
          <div className="border-t px-4 pb-4 pt-4">
            <DiffViewer diff={diff} />
          </div>
      </details>

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
