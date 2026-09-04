"use client";

import type { DemoSessionView } from "@playground/api-types";

interface Props {
  session: DemoSessionView | null;
  onReset: () => void;
  resetting: boolean;
}

function formatExpiry(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

export default function SessionBar({ session, onReset, resetting }: Props) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900 px-4 py-3 text-sm">
      <div className="flex flex-col gap-0.5">
        <span className="font-medium text-slate-200">Demo session</span>
        {session ? (
          <span className="text-slate-400">
            Expires {formatExpiry(session.expires_at)}
          </span>
        ) : (
          <span className="text-slate-500">No session yet</span>
        )}
      </div>
      <button
        type="button"
        onClick={onReset}
        disabled={resetting}
        className="rounded-md border border-slate-700 px-3 py-1.5 font-medium text-slate-200 transition hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {resetting ? "Resetting…" : "Reset session"}
      </button>
    </div>
  );
}
