"use client";

import { useEffect, useRef, useState } from "react";
import type { EventLevel, FlowEvent } from "@playground/api-types";
import { formatRelativeTime } from "@/lib/time";

interface Props {
  events: FlowEvent[];
  loading: boolean;
  error: string | null;
}

const LEVEL_STYLES: Record<EventLevel, string> = {
  info: "border-slate-800 bg-slate-900",
  success: "border-emerald-500/30 bg-emerald-500/10",
  // A rejected credential is an ordinary outcome (a wrong code, a cancelled
  // prompt) — amber, deliberately not red, so it never reads as a fault.
  rejected: "border-amber-500/30 bg-amber-500/10",
  failed: "border-red-500/30 bg-red-500/10",
};

const LEVEL_DOT: Record<EventLevel, string> = {
  info: "bg-slate-600",
  success: "bg-emerald-500",
  rejected: "bg-amber-500",
  failed: "bg-red-500",
};

const LEVEL_LABEL: Record<EventLevel, string> = {
  info: "Info",
  success: "Success",
  rejected: "Rejected",
  failed: "Failed",
};

export default function FlowLog({ events, loading, error }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // Ticks once a second so relative timestamps ("5s ago") keep advancing
  // even when no new event has arrived to force a re-render.
  const [, setTick] = useState(0);

  useEffect(() => {
    const interval = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [events.length]);

  return (
    <div className="flex h-full flex-col rounded-lg border border-slate-800 bg-slate-900">
      <div className="border-b border-slate-800 px-4 py-3">
        <h3 className="font-medium text-slate-100">Flow log</h3>
        <p className="text-xs text-slate-400">
          What the engine actually did for this visitor, as it happens.
        </p>
      </div>

      {error && (
        <p className="border-b border-amber-500/20 bg-amber-500/10 px-4 py-2 text-xs text-amber-300">
          {error}
        </p>
      )}

      <div ref={scrollRef} className="flex-1 overflow-y-auto p-4" style={{ maxHeight: 480 }}>
        {events.length === 0 ? (
          <p className="text-sm text-slate-500">
            {loading
              ? "Loading the flow log…"
              : "Nothing yet — actions you take on the left (signing in, verifying a code, registering a passkey) will appear here as they happen."}
          </p>
        ) : (
          <ol className="flex flex-col gap-2">
            {events.map((event, i) => (
              <li
                key={`${event.at}-${i}`}
                className={`rounded-md border px-3 py-2 ${LEVEL_STYLES[event.level]}`}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="flex items-center gap-1.5">
                    <span
                      aria-hidden="true"
                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${LEVEL_DOT[event.level]}`}
                    />
                    <span className="text-sm font-medium text-slate-100">{event.step}</span>
                    <span className="sr-only">({LEVEL_LABEL[event.level]})</span>
                  </div>
                  <span className="shrink-0 text-xs text-slate-500" title={event.at}>
                    {formatRelativeTime(event.at)}
                  </span>
                </div>
                <p className="mt-1 text-xs text-slate-300">{event.detail}</p>
                {event.facts.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1">
                    {event.facts.map((fact, j) => (
                      <span
                        key={`${fact.name}-${j}`}
                        className="rounded border border-slate-800 bg-slate-900 px-1.5 py-0.5 font-mono text-[11px] text-slate-300"
                      >
                        {fact.name}: {fact.value}
                      </span>
                    ))}
                  </div>
                )}
              </li>
            ))}
          </ol>
        )}
      </div>
    </div>
  );
}
