"use client";

import { useState } from "react";
import { AlertCircle, CheckCircle2, Radio, ShieldX, Signal } from "lucide-react";
import type { SsfEventResult } from "@playground/api-types";
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

const EVENT_TYPES = [
  {
    id: "https://schemas.openid.net/secevent/caep/event-type/session-revoked",
    name: "CAEP SessionRevoked (RFC 8417 / 8935)",
  },
  {
    id: "https://schemas.openid.net/secevent/caep/event-type/credential-change",
    name: "Credential Change (Password / 2FA Reset)",
  },
  {
    id: "https://schemas.openid.net/secevent/caep/event-type/token-claims-change",
    name: "Token Claims Change (Role Demotion)",
  },
];

export default function SsfPanel({ scenarioId, onDemoDisabled }: Props) {
  const [eventType, setEventType] = useState(EVENT_TYPES[0].id);
  const [subject, setSubject] = useState("user_9921_alice");
  const [reason, setReason] = useState("Device flagged as compromised by EDR agent");
  const [working, setWorking] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [result, setResult] = useState<SsfEventResult | null>(null);

  const handlePushEvent = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    const res = await scenarioAction<SsfEventResult>(scenarioId, "push_event", {
      event_type: eventType,
      subject,
      reason,
    });

    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Event push failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  const handleCheckStatus = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    const res = await scenarioAction<SsfEventResult>(scenarioId, "status", {});
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Status check failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <Label htmlFor="ssf-event-type">Security Event Type (CAEP / SSF)</Label>
        <Select
          id="ssf-event-type"
          value={eventType}
          onChange={(e) => setEventType(e.target.value)}
        >
          {EVENT_TYPES.map((e) => (
            <option key={e.id} value={e.id}>
              {e.name}
            </option>
          ))}
        </Select>
      </div>

      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <div className="grid gap-1">
          <Label htmlFor="ssf-subject" className="text-xs">Subject Identity (Subject ID)</Label>
          <Input
            id="ssf-subject"
            value={subject}
            onChange={(e) => setSubject(e.target.value)}
            className="font-mono text-xs"
          />
        </div>
        <div className="grid gap-1">
          <Label htmlFor="ssf-reason" className="text-xs">Revocation Reason</Label>
          <Input
            id="ssf-reason"
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            className="text-xs"
          />
        </div>
      </div>

      <div className="flex flex-wrap justify-end gap-2">
        <ActionButton variant="secondary" onClick={handleCheckStatus} disabled={working} className="gap-2">
          <Signal className="h-4 w-4" strokeWidth={1.5} aria-hidden />
          {working ? "Querying…" : "Check Stream"}
        </ActionButton>
        <ActionButton onClick={handlePushEvent} disabled={working} className="gap-2">
          <Radio className="h-4 w-4" strokeWidth={1.5} aria-hidden />
          {working ? "Pushing…" : "Push Security Event Token (SET)"}
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
                SET Ingestion Response
              </span>
              <Badge variant={result.session_terminated ? "destructive" : "default"}>
                HTTP {result.status_code} {result.accepted ? "Accepted" : "Rejected"}
              </Badge>
            </div>
          </div>

          <p className="text-xs text-foreground">{result.detail}</p>

          {result.session_terminated && (
            <div className="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/10 p-2.5 text-xs text-destructive-foreground">
              <ShieldX className="h-4 w-4 shrink-0 text-destructive-foreground" strokeWidth={1.5} aria-hidden />
              <span>
                <strong>Session Eviction:</strong> Live session for subject{" "}
                <code className="font-mono font-semibold">{result.subject}</code> evicted from SessionStore. Subsequent requests with existing cookies will answer 401 Unauthorized.
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
