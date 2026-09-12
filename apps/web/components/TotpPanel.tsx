"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import QRCode from "qrcode";
import { CheckCircle2, Circle, Loader2 } from "lucide-react";
import type { TotpProvision, TotpVerification } from "@playground/api-types";
import { errorDetail, scenarioAction } from "@/lib/api";
import { cn } from "@/lib/cn";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-ceremony. */
  onDemoDisabled: () => void;
  /** Called after every provision/verify round trip, so a host (e.g. the flow log) can refetch. */
}

export function normalizeCode(raw: string): string {
  // Accept spaces/dashes as visual separators (e.g. "123 456"), strip them,
  // then keep only digits and cap at 6 (TOTP codes are 6 digits).
  return raw.replace(/[\s-]/g, "").replace(/\D/g, "").slice(0, 6);
}

export default function TotpPanel({ scenarioId, onDemoDisabled }: Props) {
  const [provision, setProvision] = useState<TotpProvision | null>(null);
  const [provisioning, setProvisioning] = useState(false);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [qrError, setQrError] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);

  const [code, setCode] = useState("");
  const [verifying, setVerifying] = useState(false);
  const [verifyResult, setVerifyResult] = useState<TotpVerification | null>(null);

  const qrRequestId = useRef(0);

  useEffect(() => {
    if (!provision) {
      setQrDataUrl(null);
      setQrError(null);
      return;
    }
    const requestId = ++qrRequestId.current;
    setQrDataUrl(null);
    setQrError(null);
    QRCode.toDataURL(provision.uri, { margin: 1, width: 220 })
      .then((url) => {
        if (qrRequestId.current === requestId) setQrDataUrl(url);
      })
      .catch(() => {
        if (qrRequestId.current === requestId) {
          setQrError("Could not render a QR code. Use the secret below instead.");
        }
      });
  }, [provision]);

  const handleProvision = useCallback(async () => {
    setProvisioning(true);
    setBanner(null);
    setVerifyResult(null);
    setCode("");

    const result = await scenarioAction<TotpProvision>(scenarioId, "provision", {});

    setProvisioning(false);

    if (!result.ok) {
      switch (result.error.kind) {
        case "demo_disabled":
          onDemoDisabled();
          return;
        case "unavailable":
          setBanner("The API became unavailable while setting up the authenticator.");
          return;
        case "rate_limited":
          setBanner(result.error.detail);
          return;
        default:
          setBanner(`Could not set up the authenticator: ${errorDetail(result.error)}`);
          return;
      }
    }

    setProvision(result.data);
  }, [scenarioId, onDemoDisabled]);

  const handleVerify = useCallback(async () => {
    if (code.length !== 6 || verifying) return;

    setVerifying(true);
    setBanner(null);

    const result = await scenarioAction<TotpVerification>(scenarioId, "verify", { code });

    setVerifying(false);

    if (!result.ok) {
      switch (result.error.kind) {
        case "demo_disabled":
          onDemoDisabled();
          return;
        case "unavailable":
          setBanner("The API became unavailable while verifying the code.");
          return;
        case "rate_limited":
          setBanner(result.error.detail);
          return;
        default:
          setBanner(`Could not verify the code: ${errorDetail(result.error)}`);
          return;
      }
    }

    // verified: false is a normal outcome, not an error — render it inline.
    setVerifyResult(result.data);
  }, [scenarioId, code, verifying, onDemoDisabled]);

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="space-y-1.5">
          <CardTitle>Authenticator app (TOTP)</CardTitle>
          <CardDescription>
            Provision a secret, scan it with an authenticator app, then verify the 6-digit code it
            produces.
          </CardDescription>
        </div>
        <Badge variant={provision ? "secondary" : "outline"}>
          {provision ? "Provisioned" : "Not set up"}
        </Badge>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-2">
          <Alert role="presentation" className="border-warning/40 bg-warning/10 text-warning-foreground">
            <AlertDescription>
              Running setup again replaces the current secret — any authenticator app that already
              scanned the old QR code or secret will stop working.
            </AlertDescription>
          </Alert>
          <div>
            <Button type="button" size="sm" onClick={() => void handleProvision()} disabled={provisioning}>
              {provisioning && <Loader2 className="animate-spin" aria-hidden="true" />}
              {provisioning ? "Setting up…" : provision ? "Regenerate secret" : "Set up authenticator"}
            </Button>
          </div>
        </div>

        <div aria-live="polite" role="status">
          {banner && (
            <Alert
              role="presentation"
              className="border-warning/40 bg-warning/10 py-2 text-warning-foreground"
            >
              <AlertDescription>{banner}</AlertDescription>
            </Alert>
          )}
        </div>

        {provision && (
          <div className="flex flex-col gap-3 rounded-md border border-border bg-card p-3 sm:flex-row sm:items-start">
            {/* The QR itself only scans reliably against a light backdrop, so this
                plate is deliberately not the surrounding card colour — a padded,
                near-white surface, not a raw <img> dropped on a dark card. */}
            <div className="flex h-[220px] w-[220px] shrink-0 items-center justify-center rounded-lg bg-white p-3">
              {qrDataUrl ? (
                // eslint-disable-next-line @next/next/no-img-element -- data URL, not an app asset
                <img
                  src={qrDataUrl}
                  alt="Scan this QR code with your authenticator app"
                  width={196}
                  height={196}
                />
              ) : qrError ? (
                <span className="bg-white p-2 text-center text-xs text-background">{qrError}</span>
              ) : (
                <span className="bg-white text-xs text-background/70">Rendering…</span>
              )}
            </div>
            <div className="flex flex-1 flex-col gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                Can&apos;t scan? Enter this secret manually:
              </span>
              <code className="select-all break-all rounded border border-border bg-muted px-2 py-1 text-xs text-foreground">
                {provision.secret}
              </code>
            </div>
          </div>
        )}

        {provision && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void handleVerify();
            }}
            className="flex flex-col gap-2"
          >
            <Label htmlFor={`${scenarioId}-code`}>
              Enter the 6-digit code from your authenticator app
            </Label>
            <div className="flex items-center gap-2">
              <Input
                id={`${scenarioId}-code`}
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                maxLength={6}
                placeholder="123456"
                value={code}
                onChange={(e) => setCode(normalizeCode(e.target.value))}
                aria-invalid={verifyResult && !verifyResult.verified ? true : undefined}
                className={cn(
                  "w-28 text-center font-mono text-base tracking-[0.3em]",
                  verifyResult &&
                    !verifyResult.verified &&
                    "border-destructive focus-visible:ring-destructive",
                )}
              />
              <Button type="submit" size="sm" disabled={verifying || code.length !== 6}>
                {verifying && <Loader2 className="animate-spin" aria-hidden="true" />}
                {verifying ? "Verifying…" : "Verify"}
              </Button>
            </div>

            <div aria-live="polite" role="status">
              {verifyResult && (
                <Alert
                  role="presentation"
                  className={cn(
                    "py-2",
                    verifyResult.verified
                      ? "border-success/40 bg-success/10 text-success-foreground"
                      : "border-border bg-muted/40 text-muted-foreground",
                  )}
                >
                  <AlertDescription className="flex items-center gap-1.5">
                    {verifyResult.verified ? (
                      <CheckCircle2 className="h-4 w-4 shrink-0" aria-hidden="true" />
                    ) : (
                      <Circle className="h-4 w-4 shrink-0" aria-hidden="true" />
                    )}
                    {verifyResult.detail}
                  </AlertDescription>
                </Alert>
              )}
            </div>
          </form>
        )}
      </CardContent>
    </Card>
  );
}
