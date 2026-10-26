"use client";

import { useState } from "react";
import { AlertCircle, CheckCircle2, Key, Mail, ShieldAlert, Smartphone } from "lucide-react";
import type { PasswordlessActionResult } from "@playground/api-types";
import { errorDetail, scenarioAction } from "@/lib/api";
import { ActionButton } from "@/components/ui/action-button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";

interface Props {
  scenarioId: string;
  onDemoDisabled: () => void;
}

export default function PasswordlessPanel({ scenarioId, onDemoDisabled }: Props) {
  const [tab, setTab] = useState<"magic" | "otp" | "recovery" | "reproof">("magic");
  const [working, setWorking] = useState(false);
  const [banner, setBanner] = useState<string | null>(null);
  const [result, setResult] = useState<PasswordlessActionResult | null>(null);

  // Magic link state
  const [email, setEmail] = useState("alice@example.com");
  const [magicToken, setMagicToken] = useState("");

  // OTP state
  const [otpCode, setOtpCode] = useState("");

  // Recovery codes state
  const [recoveryCodes, setRecoveryCodes] = useState<string[]>([]);
  const [recoveryInput, setRecoveryInput] = useState("");

  const handleMagicSend = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "magic_link_send", { email });
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Magic link failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
    if (res.data.data?.token) {
      setMagicToken(res.data.data.token);
    }
  };

  const handleMagicVerify = async () => {
    if (!magicToken) return;
    setWorking(true);
    setBanner(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "magic_link_verify", { token: magicToken });
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Verification failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  const handleOtpSend = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "otp_send", {});
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`OTP send failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
    if (res.data.data?.code) {
      setOtpCode(res.data.data.code);
    }
  };

  const handleOtpVerify = async () => {
    if (!otpCode) return;
    setWorking(true);
    setBanner(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "otp_verify", { code: otpCode });
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`OTP verify failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  const handleRecoveryGenerate = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "recovery_generate", {});
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Recovery codes generation failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
    if (Array.isArray(res.data.data?.codes)) {
      setRecoveryCodes(res.data.data.codes);
      if (res.data.data.codes.length > 0) {
        setRecoveryInput(res.data.data.codes[0]);
      }
    }
  };

  const handleRecoveryConsume = async () => {
    if (!recoveryInput) return;
    setWorking(true);
    setBanner(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "recovery_consume", { code: recoveryInput });
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Recovery code consumption failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
    // Remove consumed code from state
    setRecoveryCodes((prev) => prev.filter((c) => c !== recoveryInput));
    setRecoveryInput("");
  };

  const handleReproofCheck = async () => {
    setWorking(true);
    setBanner(null);
    setResult(null);

    const res = await scenarioAction<PasswordlessActionResult>(scenarioId, "reproof_check", { max_age_seconds: 300 });
    setWorking(false);

    if (!res.ok) {
      if (res.error.kind === "demo_disabled") {
        onDemoDisabled();
        return;
      }
      setBanner(`Reproof gate check failed: ${errorDetail(res.error)}`);
      return;
    }

    setResult(res.data);
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex justify-center">
        <Tabs value={tab} onValueChange={(v) => { setTab(v as any); setResult(null); setBanner(null); }}>
          <TabsList>
            <TabsTrigger value="magic">Magic Link</TabsTrigger>
            <TabsTrigger value="otp">OTP Code</TabsTrigger>
            <TabsTrigger value="recovery">Recovery Codes</TabsTrigger>
            <TabsTrigger value="reproof">RFC-009 Freshness</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>

      {tab === "magic" && (
        <div className="flex flex-col gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="magic-email">Recipient Email Address</Label>
            <Input
              id="magic-email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="alice@example.com"
            />
          </div>

          <div className="flex justify-end gap-2">
            <ActionButton onClick={handleMagicSend} disabled={working} className="gap-2">
              <Mail className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              {working ? "Sending…" : "Send Magic Link"}
            </ActionButton>
          </div>

          {magicToken && (
            <div className="flex flex-col gap-2 rounded-lg border border-border/80 bg-muted/30 p-3">
              <Label htmlFor="magic-token">Simulated Received Token</Label>
              <Input
                id="magic-token"
                value={magicToken}
                onChange={(e) => setMagicToken(e.target.value)}
                className="font-mono text-xs"
              />
              <div className="flex justify-end pt-1">
                <ActionButton onClick={handleMagicVerify} disabled={working} className="gap-2">
                  <CheckCircle2 className="h-4 w-4" strokeWidth={1.5} aria-hidden />
                  Verify Magic Link
                </ActionButton>
              </div>
            </div>
          )}
        </div>
      )}

      {tab === "otp" && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-muted-foreground">
            Time-budgeted one-time password with NIST SP 800-63B attempt throttling (max 5 attempts).
          </p>

          <div className="flex justify-end">
            <ActionButton onClick={handleOtpSend} disabled={working} className="gap-2">
              <Smartphone className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              {working ? "Generating…" : "Generate OTP Code"}
            </ActionButton>
          </div>

          <div className="grid gap-1.5">
            <Label htmlFor="otp-input">Enter 6-digit OTP Code</Label>
            <Input
              id="otp-input"
              maxLength={6}
              value={otpCode}
              onChange={(e) => setOtpCode(e.target.value)}
              placeholder="123456"
              className="font-mono text-center tracking-widest text-sm"
            />
          </div>

          <div className="flex justify-end">
            <ActionButton onClick={handleOtpVerify} disabled={working || otpCode.length !== 6} className="gap-2">
              <CheckCircle2 className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              Verify OTP
            </ActionButton>
          </div>
        </div>
      )}

      {tab === "recovery" && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-muted-foreground">
            Emergency backup codes generated according to NIST SP 800-63B (16 single-use cryptographically random codes).
          </p>

          <div className="flex justify-end">
            <ActionButton onClick={handleRecoveryGenerate} disabled={working} className="gap-2">
              <Key className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              {working ? "Generating…" : "Generate 16 Recovery Codes"}
            </ActionButton>
          </div>

          {recoveryCodes.length > 0 && (
            <div className="rounded-lg border border-border/80 bg-muted/30 p-3">
              <span className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                Active Recovery Codes ({recoveryCodes.length} remaining)
              </span>
              <div className="mt-2 grid grid-cols-2 sm:grid-cols-4 gap-1.5 font-mono text-xs">
                {recoveryCodes.map((code) => (
                  <button
                    key={code}
                    type="button"
                    onClick={() => setRecoveryInput(code)}
                    className="rounded border border-border/60 bg-card px-2 py-1 text-center hover:border-primary transition-colors"
                  >
                    {code}
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="grid gap-1.5">
            <Label htmlFor="recovery-code-input">Redeem Single-Use Recovery Code</Label>
            <Input
              id="recovery-code-input"
              value={recoveryInput}
              onChange={(e) => setRecoveryInput(e.target.value)}
              placeholder="xxxx-xxxx-xxxx"
              className="font-mono text-xs"
            />
          </div>

          <div className="flex justify-end">
            <ActionButton
              onClick={handleRecoveryConsume}
              disabled={working || !recoveryInput}
              className="gap-2"
            >
              <CheckCircle2 className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              Consume Recovery Code
            </ActionButton>
          </div>
        </div>
      )}

      {tab === "reproof" && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-muted-foreground">
            RFC-009 Reproof Requirement: Validates freshness of authentication identity against max allowed threshold (300 seconds).
          </p>

          <div className="flex justify-end">
            <ActionButton onClick={handleReproofCheck} disabled={working} className="gap-2">
              <ShieldAlert className="h-4 w-4" strokeWidth={1.5} aria-hidden />
              {working ? "Checking…" : "Evaluate Freshness Gate"}
            </ActionButton>
          </div>
        </div>
      )}

      {banner && (
        <div className="flex items-center gap-2 rounded-lg border border-warning/40 bg-warning/10 p-3 text-xs text-warning-foreground">
          <AlertCircle className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
          <span>{banner}</span>
        </div>
      )}

      {result && (
        <div className="flex items-start gap-2.5 rounded-lg border border-border/80 bg-card p-3">
          <CheckCircle2 className="h-4 w-4 text-success-foreground shrink-0 mt-0.5" strokeWidth={1.5} aria-hidden />
          <div className="flex flex-col gap-0.5">
            <span className="text-xs font-semibold text-foreground uppercase tracking-wider">
              {result.method}: {result.success ? "Success" : "Failed"}
            </span>
            <p className="text-xs text-muted-foreground">{result.detail}</p>
          </div>
        </div>
      )}
    </div>
  );
}
