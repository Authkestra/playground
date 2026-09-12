"use client";

import { cn } from "@/lib/cn";

export interface WizardStep {
  id: 1 | 2 | 3;
  title: string;
}

export const WIZARD_STEPS: WizardStep[] = [
  { id: 1, title: "Choose sign-in methods" },
  { id: 2, title: "Sign in" },
  { id: 3, title: "Download" },
];

interface Props {
  current: 1 | 2 | 3;
  /** Highest step the visitor has reached — steps up to here are navigable back. */
  maxReached: 1 | 2 | 3;
  onNavigate: (step: 1 | 2 | 3) => void;
}

export default function StepIndicator({ current, maxReached, onNavigate }: Props) {
  return (
    <nav aria-label="Wizard steps">
      <ol className="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-0">
        {WIZARD_STEPS.map((step, i) => {
          const isCurrent = step.id === current;
          const isComplete = step.id < current;
          const isNavigable = step.id <= maxReached && step.id !== current;

          return (
            <li key={step.id} className="flex flex-1 items-center gap-2 sm:gap-3">
              {isNavigable ? (
                <button
                  type="button"
                  aria-current={isCurrent ? "step" : undefined}
                  onClick={() => onNavigate(step.id)}
                  className="flex items-center gap-2.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-accent"
                >
                  <StepBadge id={step.id} isCurrent={isCurrent} isComplete={isComplete} />
                  <StepLabel title={step.title} isCurrent={isCurrent} />
                </button>
              ) : (
                <div
                  aria-current={isCurrent ? "step" : undefined}
                  className="flex items-center gap-2.5 px-2 py-1.5"
                >
                  <StepBadge id={step.id} isCurrent={isCurrent} isComplete={isComplete} />
                  <StepLabel title={step.title} isCurrent={isCurrent} />
                </div>
              )}
              {i < WIZARD_STEPS.length - 1 && (
                <div
                  aria-hidden="true"
                  className={cn(
                    "hidden h-px flex-1 sm:block",
                    step.id < current ? "bg-primary/60" : "bg-border",
                  )}
                />
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}

function StepBadge({
  id,
  isCurrent,
  isComplete,
}: {
  id: number;
  isCurrent: boolean;
  isComplete: boolean;
}) {
  return (
    <span
      className={cn(
        "flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-xs font-semibold transition-colors",
        isCurrent
          ? "bg-primary text-primary-foreground"
          : isComplete
            ? "bg-secondary text-secondary-foreground"
            : "bg-muted text-muted-foreground",
      )}
    >
      {isComplete ? "✓" : id}
    </span>
  );
}

function StepLabel({ title, isCurrent }: { title: string; isCurrent: boolean }) {
  return (
    <span className={cn("text-sm", isCurrent ? "font-semibold text-foreground" : "text-muted-foreground")}>
      {title}
    </span>
  );
}
