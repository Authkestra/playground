"use client";

import { useState } from "react";
import { AlertCircle, Bot, CheckCircle2, KeyRound, ShieldAlert, Terminal } from "lucide-react";
import type { McpInvocationResult } from "@playground/api-types";
import { errorDetail, scenarioAction } from "@/lib/api";
import { ActionButton } from "@/components/ui/action-button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";

interface Props {
  scenarioId: string;
  onDemoDisabled: () => void;
}

const TOOLS = [
  { id: "execute_wire_transfer", name: "execute_wire_transfer (Tier 3: HumanApproved)", tier: "HumanApproved" },
  { id: "read_financial_records", name: "read_financial_records (Tier 2: Scoped)", tier: "Scoped" },
  { id: "ping_health", name: "ping_health (Tier 1: Public)", tier: "Public" },
];

export default function McpPanel({ scenarioId, onDemoDisabled }: Props) {
  const [tool, setTool] = useState("execute_wire_transfer");
  const [paramsText, setParamsText] = useState(
    JSON.stringify({ amount: 50000, destination: "treasury-corp-4" }, null, 2),
  );
  const [invoking, setInvoking] = useState(false);
  const [approving, setApproving] = useState(false);
  const [tampering, setTampering] = useState(false);
  const [result, setResult] = useState<McpInvocationResult | null>(null);
  const [banner, setBanner] = useState<string | null>(null);

  const handleToolChange = (newTool: string) => {
    setTool(newTool);
    setResult(null);
    setBanner(null);
    if (newTool === "execute_wire_transfer") {
      setParamsText(JSON.stringify({ amount: 50000, destination: "treasury-corp-4" }, null, 2));
    } else if (newTool === "read_financial_records") {
      setParamsText(JSON.stringify({ account_id: "corp-8821", limit: 10 }, null, 2));
    } else {
      setParamsText(JSON.stringify({ ping: "now" }, null, 2));
    }
  };

  const handleInvoke = async () => {
    setInvoking(true);
    setBanner(null);
    setResult(null);

    let parsedParams: unknown;
    try {
      parsedParams = JSON.parse(paramsText);
    } catch {
      setInvoking(false);
      setBanner("Invalid JSON parameters format.");
      return;
    }

    const res = await scenarioAction<McpInvocationResult>(scenarioId, "invoke", {
      tool_name: tool,
      arguments: parsedParams,
    });

    setInvoking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Tool invocation failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  const handleApprove = async () => {
    if (!result?.challenge_id || !result?.digest) return;
    setApproving(true);
    setBanner(null);

    const res = await scenarioAction<McpInvocationResult>(scenarioId, "approve", {
      challenge_id: result.challenge_id,
      digest: result.digest,
    });

    setApproving(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Biometric approval failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  const handleExecute = async () => {
    if (!result?.execution_token || !result?.digest) return;
    setApproving(true);
    setBanner(null);

    let parsedParams: unknown;
    try {
      parsedParams = JSON.parse(paramsText);
    } catch {
      // ignore
    }

    const res = await scenarioAction<McpInvocationResult>(scenarioId, "execute", {
      execution_token: result.execution_token,
      digest: result.digest,
      tool_name: tool,
      arguments: parsedParams,
    });

    setApproving(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Execution failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  const handleTamper = async () => {
    if (!result?.execution_token) return;
    setTampering(true);
    setBanner(null);

    let parsedParams: unknown;
    try {
      parsedParams = JSON.parse(paramsText);
    } catch {
      // ignore
    }

    const res = await scenarioAction<McpInvocationResult>(scenarioId, "tamper", {
      execution_token: result.execution_token,
      tool_name: tool,
      arguments: parsedParams,
    });

    setTampering(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Tamper test response: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="mcp-tool-select">Select MCP Tool</Label>
        <Select
          id="mcp-tool-select"
          value={tool}
          onChange={(e) => handleToolChange(e.target.value)}
        >
          {TOOLS.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </Select>
      </div>

      <div className="flex flex-col gap-1.5">
        <Label htmlFor="mcp-params">Tool Invocation Parameters (JSON)</Label>
        <textarea
          id="mcp-params"
          rows={3}
          value={paramsText}
          onChange={(e) => setParamsText(e.target.value)}
          className="w-full rounded-md border border-input bg-background px-3 py-2 font-mono text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
      </div>

      <div className="flex justify-end">
        <ActionButton onClick={handleInvoke} disabled={invoking} className="gap-2">
          <Terminal className="h-4 w-4" strokeWidth={1.5} aria-hidden />
          {invoking ? "Invoking…" : "Invoke MCP Tool"}
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
              <Bot className="h-4 w-4 text-primary-accent" strokeWidth={1.5} aria-hidden />
              <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                Invocation Result
              </span>
            </div>
            <Badge
              variant={
                result.status === "step_up_required" || result.status === "challenge"
                  ? "outline"
                  : result.status === "completed" || result.status === "executed"
                    ? "default"
                    : "destructive"
              }
            >
              {result.status === "challenge" ? "step_up_required" : result.status}
            </Badge>
          </div>

          <p className="text-xs text-foreground">{result.detail}</p>

          {result.digest && (
            <div className="rounded border border-border/60 bg-muted/40 p-2 font-mono text-[11px] text-muted-foreground break-all">
              <span className="font-semibold text-foreground">RFC 8785 Canonical Digest (SHA-256): </span>
              {result.digest}
            </div>
          )}

          {(result.status === "step_up_required" || result.status === "challenge") && (
            <div className="flex flex-col gap-3 rounded-lg border border-primary/30 bg-primary/5 p-3">
              <div className="flex items-center gap-2 text-xs font-medium text-foreground">
                <ShieldAlert className="h-4 w-4 text-primary-accent" strokeWidth={1.5} aria-hidden />
                <span>MCP Protocol Step-Up Required (Error -32001)</span>
              </div>
              <p className="text-xs text-muted-foreground">
                This action requires interactive human approval. Parameter binding ensures what you sign is exactly what executes.
              </p>
              <div className="flex flex-wrap justify-end gap-2 pt-1">
                <ActionButton
                  onClick={handleApprove}
                  disabled={approving}
                  className="gap-2"
                >
                  <KeyRound className="h-4 w-4" strokeWidth={1.5} aria-hidden />
                  {approving ? "Approving…" : "Approve with Biometric Passkey"}
                </ActionButton>
              </div>
            </div>
          )}

          {result.execution_token && result.status !== "executed" && (
            <div className="flex flex-col gap-2 rounded-lg border border-border/60 bg-muted/30 p-3">
              <div className="flex items-center gap-2 text-xs font-medium text-foreground">
                <CheckCircle2 className="h-4 w-4 text-success-foreground" strokeWidth={1.5} aria-hidden />
                <span>Single-Use Execution Token Issued</span>
              </div>
              <div className="flex flex-wrap justify-end gap-2 pt-1">
                <ActionButton
                  onClick={handleExecute}
                  disabled={approving}
                  className="gap-2"
                >
                  {approving ? "Executing…" : "Execute Action"}
                </ActionButton>
                <ActionButton
                  variant="outline"
                  onClick={handleTamper}
                  disabled={tampering}
                  className="gap-2"
                >
                  {tampering ? "Testing…" : "Test Tampered Parameters (Rejection Demo)"}
                </ActionButton>
              </div>
            </div>
          )}

          {result.output && (
            <div className="rounded border border-border/60 bg-muted/30 p-2">
              <span className="text-[11px] font-semibold text-muted-foreground">Execution Output:</span>
              <pre className="mt-1 overflow-x-auto font-mono text-[11px] text-foreground">
                {JSON.stringify(result.output, null, 2)}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
