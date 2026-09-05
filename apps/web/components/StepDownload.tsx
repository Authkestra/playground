"use client";

import { useState } from "react";
import type { DemoConfig, ScenarioSpec } from "@playground/api-types";
import { downloadStarterKit, type ApiError } from "@/lib/api";
import { isControlValueActive } from "@/components/ScenarioPanel";

interface Props {
  scenarios: ScenarioSpec[];
  config: DemoConfig | null;
  onDemoDisabled: () => void;
  onBack: () => void;
}

const STAR_URL = "https://github.com/marcjazz/authkestra";

type State =
  | { kind: "idle" }
  | { kind: "working" }
  | { kind: "done"; filename: string }
  | { kind: "failed"; message: string };

export default function StepDownload({
  scenarios,
  config,
  onDemoDisabled,
  onBack,
}: Props) {
  const [state, setState] = useState<State>({ kind: "idle" });

  const included = scenarios.filter((s) => {
    const value = config?.scenarios?.[s.id];
    return value ? isControlValueActive(value) : false;
  });

  async function handleDownload() {
    setState({ kind: "working" });
    const result = await downloadStarterKit();

    if (!result.ok) {
      if (result.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setState({ kind: "failed", message: describe(result.error) });
      return;
    }

    const { blob, filename } = result.data;
    // Hand the bytes to the browser's own save flow. The object URL is revoked
    // straight after: it pins the blob in memory until it is.
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);

    setState({ kind: "done", filename });
  }

  const working = state.kind === "working";

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-slate-100">Download</h2>
        <p className="text-sm text-slate-400">
          Turn what you configured into a real, runnable project.
        </p>
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-6">
        <h3 className="text-sm font-medium text-slate-200">What you&apos;ll get</h3>

        {included.length > 0 ? (
          <ul className="mt-3 flex flex-col gap-1.5 text-sm text-slate-300">
            {included.map((s) => (
              <li key={s.id} className="flex items-start gap-2">
                <span aria-hidden className="mt-0.5 text-emerald-400">
                  &#10003;
                </span>
                <span>{s.name}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-3 text-sm text-slate-400">
            You haven&apos;t turned anything on, so this is the smallest project
            that still runs: sessions and the framework&apos;s{" "}
            <code className="rounded bg-slate-800 px-1 py-0.5 font-mono text-xs">
              /auth
            </code>{" "}
            routes, ready for you to add a method to.
          </p>
        )}

        <p className="mt-4 text-sm text-slate-400">
          A Cargo project pinned to the same authkestra version this playground
          runs, with a README that names every value you need to fill in and
          where to get it. No sign-up, no gate.
        </p>

        <button
          type="button"
          onClick={() => void handleDownload()}
          disabled={working}
          className="mt-5 rounded-md bg-emerald-500 px-4 py-2 text-sm font-semibold text-slate-950 transition hover:bg-emerald-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-emerald-300 focus-visible:ring-offset-2 focus-visible:ring-offset-slate-900 disabled:cursor-not-allowed disabled:bg-emerald-500/50"
        >
          {working ? "Preparing…" : "Download the project"}
        </button>

        <div aria-live="polite" className="mt-3 min-h-[1.25rem]">
          {state.kind === "done" && (
            <p className="text-sm text-emerald-400">
              Saved{" "}
              <code className="rounded bg-slate-800 px-1 py-0.5 font-mono text-xs text-emerald-300">
                {state.filename}
              </code>
              . Unzip it, then follow the README.
            </p>
          )}
          {state.kind === "failed" && (
            <p className="text-sm text-amber-300">{state.message}</p>
          )}
        </div>
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900/50 p-4">
        <p className="text-sm text-slate-400">
          If this saved you time, a star on{" "}
          <a
            href={STAR_URL}
            target="_blank"
            rel="noreferrer noopener"
            className="font-medium text-slate-200 underline underline-offset-2 hover:text-white"
          >
            marcjazz/authkestra
          </a>{" "}
          helps other people find it. Entirely optional, and never a condition
          of the download.
        </p>
      </div>

      <div>
        <button
          type="button"
          onClick={onBack}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-sm font-medium text-slate-200 transition hover:bg-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-400 focus-visible:ring-offset-2"
        >
          Back
        </button>
      </div>
    </div>
  );
}

function describe(error: ApiError): string {
  switch (error.kind) {
    case "unavailable":
      return "Couldn't reach the API. Check your connection and try again.";
    case "rate_limited":
      return error.detail;
    case "demo_disabled":
      // Handled by the caller, which switches the whole page into explainer
      // mode rather than reporting it here.
      return "Live flows are switched off right now.";
    case "state_unavailable":
      // Distinct from `demo_disabled` on purpose: the demo is not switched
      // off, the store behind it is unreachable. Saying "temporarily" is the
      // honest difference — this one is worth retrying, and it is an outage
      // rather than an intentional state.
      return "The playground's state store is temporarily unreachable, so the project could not be generated. Try again in a moment.";
    case "http_error":
      return `The download failed (${error.status}). ${error.detail}`;
  }
}
