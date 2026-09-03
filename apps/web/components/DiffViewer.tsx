import type { ConfigDiff, DiffKind } from "@playground/api-types";

interface Props {
  diff: ConfigDiff | null;
}

const KIND_STYLES: Record<DiffKind, string> = {
  added: "border-emerald-200 bg-emerald-50 text-emerald-700",
  removed: "border-red-200 bg-red-50 text-red-700",
  changed: "border-amber-200 bg-amber-50 text-amber-700",
};

const KIND_SYMBOL: Record<DiffKind, string> = {
  added: "+",
  removed: "−",
  changed: "~",
};

export default function DiffViewer({ diff }: Props) {
  if (!diff) {
    return (
      <p className="text-sm text-slate-400">
        Configure a scenario above to see how it changes the config.
      </p>
    );
  }

  const { entries, consequences } = diff;
  const hasConsequences =
    consequences.routes.length > 0 ||
    consequences.requirements.length > 0 ||
    consequences.crates.length > 0;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1 font-mono text-xs">
        {entries.length === 0 ? (
          <span className="text-slate-400">No changes.</span>
        ) : (
          entries.map((entry, i) => (
            <div
              key={`${entry.path}-${i}`}
              className={`flex items-start gap-2 rounded border px-2 py-1 ${KIND_STYLES[entry.kind]}`}
            >
              <span className="font-bold">{KIND_SYMBOL[entry.kind]}</span>
              <span className="flex-1 break-all">
                <span className="font-semibold">{entry.path}</span>
                {entry.before !== null && <span> {entry.before} →</span>}
                {entry.after !== null && <span> {entry.after}</span>}
              </span>
            </div>
          ))
        )}
      </div>

      {hasConsequences && (
        <div className="grid gap-4 border-t border-slate-100 pt-3 sm:grid-cols-3">
          <ConsequenceList title="Routes" items={consequences.routes} />
          <ConsequenceList title="Requirements" items={consequences.requirements} />
          <div>
            <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-slate-500">
              Crates
            </h4>
            {consequences.crates.length === 0 ? (
              <p className="text-xs text-slate-400">None</p>
            ) : (
              <ul className="flex flex-col gap-1 text-xs text-slate-600">
                {consequences.crates.map((c) => (
                  <li key={c.name}>
                    <span className="font-mono">{c.name}</span>
                    {c.features.length > 0 && (
                      <span className="text-slate-400"> [{c.features.join(", ")}]</span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function ConsequenceList({ title, items }: { title: string; items: string[] }) {
  return (
    <div>
      <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-slate-500">
        {title}
      </h4>
      {items.length === 0 ? (
        <p className="text-xs text-slate-400">None</p>
      ) : (
        <ul className="flex flex-col gap-1 text-xs text-slate-600">
          {items.map((item) => (
            <li key={item} className="font-mono">
              {item}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
