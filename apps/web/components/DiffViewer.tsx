import type { ConfigDiff, DiffKind } from "@playground/api-types";
import { Badge } from "@/components/ui/badge";

interface Props {
  diff: ConfigDiff | null;
}

const KIND_STYLES: Record<DiffKind, string> = {
  added: "border-success/30 bg-success/10 text-success-foreground",
  removed: "border-destructive/30 bg-destructive/10 text-destructive-foreground",
  changed: "border-warning/30 bg-warning/10 text-warning-foreground",
};

const KIND_SYMBOL: Record<DiffKind, string> = {
  added: "+",
  removed: "−",
  changed: "~",
};

export default function DiffViewer({ diff }: Props) {
  if (!diff) {
    return (
      <p className="text-sm text-muted-foreground">
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
          <span className="text-muted-foreground">No changes.</span>
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
        <div className="grid gap-4 border-t border-border pt-3 sm:grid-cols-3">
          <ConsequenceList title="Routes" items={consequences.routes} />
          <ConsequenceList title="Requirements" items={consequences.requirements} />
          <div>
            <h4 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Crates
            </h4>
            {consequences.crates.length === 0 ? (
              <p className="text-xs text-muted-foreground">None</p>
            ) : (
              <ul className="flex flex-col gap-1.5">
                {consequences.crates.map((c) => (
                  <li key={c.name} className="break-words">
                    <Badge variant="outline" className="font-mono font-normal">
                      {c.name}
                    </Badge>
                    {c.features.length > 0 && (
                      <span className="ml-1 text-xs text-muted-foreground">
                        [{c.features.join(", ")}]
                      </span>
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
      <h4 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h4>
      {items.length === 0 ? (
        <p className="text-xs text-muted-foreground">None</p>
      ) : (
        <ul className="flex flex-wrap gap-1.5">
          {items.map((item) => (
            <li key={item} className="break-all">
              <Badge variant="outline" className="font-mono font-normal">
                {item}
              </Badge>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
