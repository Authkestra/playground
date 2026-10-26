"use client";

import { useState } from "react";
import { AlertCircle, CheckCircle2, FileCheck, ShieldAlert, Zap } from "lucide-react";
import type { PolicyEvaluationResult } from "@playground/api-types";
import { errorDetail, scenarioAction } from "@/lib/api";
import { ActionButton } from "@/components/ui/action-button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";

interface Props {
  scenarioId: string;
  onDemoDisabled: () => void;
}

const TEMPLATES = [
  { id: "department_abac", name: "Department & Clearance (ABAC)" },
  { id: "admin_only", name: "Admin Role (Strict RBAC)" },
  { id: "emergency_break_glass", name: "Emergency Break-Glass (Context ReBAC)" },
];

export default function PolicyPanel({ scenarioId, onDemoDisabled }: Props) {
  const [template, setTemplate] = useState("department_abac");
  const [principal, setPrincipal] = useState('User::"alice"');
  const [actionUid, setActionUid] = useState('Action::"readDocument"');
  const [resource, setResource] = useState('Document::"q3_roadmap"');
  const [contextText, setContextText] = useState(
    JSON.stringify({ department: "engineering", clearance: "confidential", untrusted_ip: false }, null, 2),
  );
  const [working, setWorking] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [result, setResult] = useState<PolicyEvaluationResult | null>(null);

  const handleTemplateChange = (t: string) => {
    setTemplate(t);
    setResult(null);
    setBanner(null);
    if (t === "admin_only") {
      setPrincipal('User::"alice"');
      setActionUid('Action::"deleteServer"');
      setResource('Server::"prod-1"');
      setContextText(JSON.stringify({ is_admin: true, is_anonymous: false }, null, 2));
    } else if (t === "emergency_break_glass") {
      setPrincipal('User::"bob"');
      setActionUid('Action::"patchDatabase"');
      setResource('Database::"billing"');
      setContextText(JSON.stringify({ break_glass: true, justification: "P1 Production Outage" }, null, 2));
    } else {
      setPrincipal('User::"alice"');
      setActionUid('Action::"readDocument"');
      setResource('Document::"q3_roadmap"');
      setContextText(JSON.stringify({ department: "engineering", clearance: "confidential", untrusted_ip: false }, null, 2));
    }
  };

  const handleEvaluate = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    let parsedContext: unknown;
    try {
      parsedContext = JSON.parse(contextText);
    } catch {
      setWorking(false);
      setBanner("Invalid JSON context format.");
      return;
    }

    const res = await scenarioAction<PolicyEvaluationResult>(scenarioId, "evaluate", {
      principal,
      action: actionUid,
      resource,
      context: parsedContext,
    });

    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Evaluation failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="policy-template-select">Cedar Policy Rule Template</Label>
        <Select
          id="policy-template-select"
          value={template}
          onChange={(e) => handleTemplateChange(e.target.value)}
        >
          {TEMPLATES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </Select>
      </div>

      <div className="grid grid-cols-1 sm:grid-cols-3 gap-2">
        <div className="grid gap-1">
          <Label htmlFor="principal-uid" className="text-xs">Principal Entity UID</Label>
          <Input
            id="principal-uid"
            value={principal}
            onChange={(e) => setPrincipal(e.target.value)}
            className="font-mono text-xs"
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor="action-uid" className="text-xs">Action Entity UID</Label>
          <Input
            id="action-uid"
            value={actionUid}
            onChange={(e) => setActionUid(e.target.value)}
            className="font-mono text-xs"
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor="resource-uid" className="text-xs">Resource Entity UID</Label>
          <Input
            id="resource-uid"
            value={resource}
            onChange={(e) => setResource(e.target.value)}
            className="font-mono text-xs"
          />
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label htmlFor="context-json">Request Context (JSON)</Label>
        <textarea
          id="context-json"
          rows={3}
          value={contextText}
          onChange={(e) => setContextText(e.target.value)}
          className="w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
      </div>

      <div className="flex justify-end">
        <ActionButton onClick={handleEvaluate} disabled={working} className="gap-2">
          <FileCheck className="h-4 w-4" strokeWidth={1.5} aria-hidden />
          {working ? "Evaluating…" : "Evaluate Cedar Policy"}
        </ActionButton>
      </div>

      {banner && (
        <div className="flex items-center gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-xs text-warning-foreground">
          <AlertCircle className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
          <span>{banner}</span>
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-3 rounded-lg border border-border/80 bg-card p-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                Cedar Verdict
              </span>
              <Badge variant={result.allowed ? "default" : "destructive"}>
                {result.decision}
              </Badge>
            </div>
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Zap className="h-3.5 w-3.5 text-primary-accent" strokeWidth={1.5} aria-hidden />
              <span>{result.latency_micros.toString()} µs decision time</span>
            </div>
          </div>

          <p className="text-xs text-foreground">{result.detail}</p>

          {result.diagnostics.length > 0 && (
            <div className="flex flex-col gap-1 rounded border border-border/60 bg-muted/30 p-2 text-xs">
              <span className="text-[11px] font-semibold text-muted-foreground">Satisfied Rule Clauses:</span>
              <ul className="list-inside list-disc font-mono text-[11px] text-foreground">
                {result.diagnostics.map((diag) => (
                  <li key={diag}>{diag}</li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
