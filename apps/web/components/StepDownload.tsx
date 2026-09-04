"use client";

interface Props {
  onBack: () => void;
}

export default function StepDownload({ onBack }: Props) {
  return (
    <div className="flex flex-col gap-6">
      <div>
        <h2 className="text-lg font-semibold text-slate-800">Download</h2>
        <p className="text-sm text-slate-500">
          Turn what you configured into a real, runnable project.
        </p>
      </div>

      <div className="rounded-lg border border-dashed border-slate-300 bg-white p-8 text-center">
        <p className="text-sm font-medium text-slate-700">This step isn&apos;t built yet.</p>
        <p className="mx-auto mt-2 max-w-md text-sm text-slate-500">
          Once available, you&apos;ll get a ready-to-run Cargo project matching the
          sign-in methods you chose in step 1 &mdash; no sign-up, no gate.
        </p>
        <p className="mx-auto mt-2 max-w-md text-sm text-slate-500">
          If it saves you time, a star on{" "}
          <code className="rounded bg-slate-100 px-1 py-0.5 font-mono text-xs">
            marcjazz/authkestra
          </code>{" "}
          is appreciated &mdash; entirely optional, and never a condition of the
          download.
        </p>
        <button
          type="button"
          disabled
          title="Not built yet"
          className="mt-5 cursor-not-allowed rounded-md border border-slate-300 bg-slate-100 px-4 py-2 text-sm font-medium text-slate-400"
        >
          Download (coming soon)
        </button>
      </div>

      <div>
        <button
          type="button"
          onClick={onBack}
          className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 transition hover:bg-slate-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-slate-500 focus-visible:ring-offset-2"
        >
          Back
        </button>
      </div>
    </div>
  );
}
