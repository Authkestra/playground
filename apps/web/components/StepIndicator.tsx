"use client";

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
                  className="flex items-center gap-2 rounded-md px-1 py-1 text-left transition hover:bg-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2"
                >
                  <StepBadge id={step.id} isCurrent={isCurrent} isComplete={isComplete} />
                  <StepLabel title={step.title} isCurrent={isCurrent} />
                </button>
              ) : (
                <div aria-current={isCurrent ? "step" : undefined} className="flex items-center gap-2 px-1 py-1">
                  <StepBadge id={step.id} isCurrent={isCurrent} isComplete={isComplete} />
                  <StepLabel title={step.title} isCurrent={isCurrent} />
                </div>
              )}
              {i < WIZARD_STEPS.length - 1 && (
                <div
                  aria-hidden="true"
                  className={`hidden h-px flex-1 sm:block ${
                    step.id < current ? "bg-slate-600" : "bg-slate-700"
                  }`}
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
      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-xs font-semibold ${
        isCurrent
          ? "bg-slate-200 text-slate-900"
          : isComplete
            ? "bg-slate-700 text-slate-200"
            : "bg-slate-800 text-slate-500"
      }`}
    >
      {isComplete ? "✓" : id}
    </span>
  );
}

function StepLabel({ title, isCurrent }: { title: string; isCurrent: boolean }) {
  return (
    <span className={`text-sm ${isCurrent ? "font-semibold text-slate-100" : "text-slate-400"}`}>
      {title}
    </span>
  );
}
