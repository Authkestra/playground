"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import QRCode from "qrcode";
import type { TotpProvision, TotpVerification } from "@playground/api-types";
import { scenarioAction } from "@/lib/api";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-ceremony. */
  onDemoDisabled: () => void;
}

function normalizeCode(raw: string): string {
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
          setBanner(`Could not set up the authenticator: ${result.error.detail}`);
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
          setBanner(`Could not verify the code: ${result.error.detail}`);
          return;
      }
    }

    // verified: false is a normal outcome, not an error — render it inline.
    setVerifyResult(result.data);
  }, [scenarioId, code, verifying, onDemoDisabled]);

  return (
    <div className="flex flex-col gap-4">
      <div>
        <p className="text-xs text-amber-600">
          Running setup again replaces the current secret — any authenticator
          app that already scanned the old QR code or secret will stop
          working.
        </p>
        <button
          type="button"
          onClick={() => void handleProvision()}
          disabled={provisioning}
          className="mt-2 rounded-md border border-slate-300 px-2.5 py-1 text-xs font-medium text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {provisioning
            ? "Setting up…"
            : provision
              ? "Regenerate secret"
              : "Set up authenticator"}
        </button>
      </div>

      {banner && <p className="text-xs text-amber-600">{banner}</p>}

      {provision && (
        <div className="flex flex-col gap-3 rounded-md border border-slate-100 bg-slate-50 p-3 sm:flex-row sm:items-start">
          <div className="flex h-[220px] w-[220px] shrink-0 items-center justify-center rounded bg-white">
            {qrDataUrl ? (
              // eslint-disable-next-line @next/next/no-img-element -- data URL, not an app asset
              <img
                src={qrDataUrl}
                alt="Scan this QR code with your authenticator app"
                width={220}
                height={220}
              />
            ) : qrError ? (
              <span className="p-2 text-center text-xs text-amber-600">{qrError}</span>
            ) : (
              <span className="text-xs text-slate-400">Rendering…</span>
            )}
          </div>
          <div className="flex flex-1 flex-col gap-1">
            <span className="text-xs font-medium text-slate-600">
              Can&apos;t scan? Enter this secret manually:
            </span>
            <code className="select-all break-all rounded border border-slate-200 bg-white px-2 py-1 text-xs text-slate-700">
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
          <label className="text-xs font-medium text-slate-600" htmlFor={`${scenarioId}-code`}>
            Enter the 6-digit code from your authenticator app
          </label>
          <div className="flex items-center gap-2">
            <input
              id={`${scenarioId}-code`}
              type="text"
              inputMode="numeric"
              autoComplete="one-time-code"
              placeholder="123456"
              value={code}
              onChange={(e) => setCode(normalizeCode(e.target.value))}
              className="w-32 rounded-md border border-slate-300 px-2 py-1 text-sm font-mono tracking-widest text-slate-800 focus:border-slate-500 focus:outline-none"
            />
            <button
              type="submit"
              disabled={verifying || code.length !== 6}
              className="rounded-md border border-slate-300 px-2.5 py-1 text-xs font-medium text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {verifying ? "Verifying…" : "Verify"}
            </button>
          </div>

          {verifyResult && (
            <p
              className={`flex items-center gap-1.5 text-xs ${
                verifyResult.verified ? "text-emerald-600" : "text-slate-600"
              }`}
            >
              <span aria-hidden="true">{verifyResult.verified ? "✓" : "•"}</span>
              {verifyResult.detail}
            </p>
          )}
        </form>
      )}
    </div>
  );
}
