"use client";

import { useCallback, useEffect, useState } from "react";
import { CheckCircle2, Circle, Loader2 } from "lucide-react";
import type { PasskeyAuthResult, PasskeyEnrolment } from "@playground/api-types";
import { scenarioAction, type ApiError } from "@/lib/api";
import { cn } from "@/lib/cn";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Separator } from "@/components/ui/separator";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-ceremony. */
  onDemoDisabled: () => void;
  /** Called after every register/authenticate round trip, so a host (e.g. the flow log) can refetch. */
}

// ---------------------------------------------------------------------------
// Wire shapes (base64url) vs. browser shapes (ArrayBuffer)
//
// The server serialises WebAuthn options/credentials as JSON, so binary
// fields (challenge, credential ids, signatures, ...) travel as base64url
// strings. `navigator.credentials.create()/get()` instead wants and returns
// real ArrayBuffers. Everything below exists to cross that boundary exactly
// once in each direction.
// ---------------------------------------------------------------------------

interface RawCredentialDescriptor {
  id: string;
  type: "public-key";
  transports?: AuthenticatorTransport[];
}

interface RawCreationOptions
  extends Omit<PublicKeyCredentialCreationOptions, "challenge" | "user" | "excludeCredentials"> {
  challenge: string;
  user: { id: string; name: string; displayName: string };
  excludeCredentials?: RawCredentialDescriptor[];
}

interface RawRequestOptions
  extends Omit<PublicKeyCredentialRequestOptions, "challenge" | "allowCredentials"> {
  challenge: string;
  allowCredentials?: RawCredentialDescriptor[];
}

interface RegisterStartResponse {
  publicKey: RawCreationOptions;
}

interface AuthenticateStartResponse {
  publicKey: RawRequestOptions;
}

/**
 * base64url -> Uint8Array.
 *
 * base64url (RFC 4648 §5) is plain base64 with `-`/`_` in place of `+`/`/`
 * and no `=` padding, so it can travel safely inside URLs and JSON strings
 * without escaping. `atob` only understands the padded, `+`/`/` alphabet, so
 * we translate the alphabet back and restore padding to a multiple of 4
 * before decoding.
 */
export function base64UrlToBuffer(value: string): Uint8Array<ArrayBuffer> {
  const base64 = value.replace(/-/g, "+").replace(/_/g, "/");
  const padLength = (4 - (base64.length % 4)) % 4;
  const padded = base64 + "=".repeat(padLength);
  const binary = atob(padded);
  const buffer = new ArrayBuffer(binary.length);
  const bytes = new Uint8Array(buffer);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** ArrayBuffer -> base64url (see base64UrlToBuffer for the alphabet note). */
export function bufferToBase64Url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let i = 0; i < bytes.length; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  const base64 = btoa(binary);
  return base64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function decodeCredentialDescriptors(
  descriptors: RawCredentialDescriptor[] | undefined,
): PublicKeyCredentialDescriptor[] | undefined {
  return descriptors?.map((cred) => ({
    ...cred,
    id: base64UrlToBuffer(cred.id),
  }));
}

function decodeCreationOptions(raw: RawCreationOptions): PublicKeyCredentialCreationOptions {
  return {
    ...raw,
    challenge: base64UrlToBuffer(raw.challenge),
    user: { ...raw.user, id: base64UrlToBuffer(raw.user.id) },
    excludeCredentials: decodeCredentialDescriptors(raw.excludeCredentials),
  };
}

function decodeRequestOptions(raw: RawRequestOptions): PublicKeyCredentialRequestOptions {
  return {
    ...raw,
    challenge: base64UrlToBuffer(raw.challenge),
    allowCredentials: decodeCredentialDescriptors(raw.allowCredentials),
  };
}

function encodeAttestation(credential: PublicKeyCredential): unknown {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
      attestationObject: bufferToBase64Url(response.attestationObject),
    },
  };
}

function encodeAssertion(credential: PublicKeyCredential): unknown {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
      authenticatorData: bufferToBase64Url(response.authenticatorData),
      signature: bufferToBase64Url(response.signature),
      userHandle: response.userHandle ? bufferToBase64Url(response.userHandle) : null,
    },
  };
}

// ---------------------------------------------------------------------------
// Capability detection
// ---------------------------------------------------------------------------

type Capability = "checking" | "unsupported" | "supported";
type UnsupportedReason = "no-webauthn" | "insecure-context";
type AdvisoryReason = "no-platform-authenticator";

const UNSUPPORTED_MESSAGES: Record<UnsupportedReason, string> = {
  "no-webauthn":
    "This browser doesn't support passkeys (the WebAuthn API isn't available). " +
    "Try the \"Authenticator app (TOTP)\" scenario instead — it works everywhere.",
  "insecure-context":
    "Passkeys require a secure context (HTTPS), and this page isn't loaded over one. " +
    "Try the \"Authenticator app (TOTP)\" scenario instead.",
};

const ADVISORY_MESSAGES: Record<AdvisoryReason, string> = {
  "no-platform-authenticator":
    "This device doesn't have a built-in authenticator (like Touch ID, Windows Hello, or a fingerprint sensor). " +
    "When registering or authenticating, the browser will offer a security key or a nearby phone instead.",
};

/** Narrow, optional-safe view of the bits of `PublicKeyCredential` we probe. */
type PublicKeyCredentialStatics = {
  isUserVerifyingPlatformAuthenticatorAvailable?: () => Promise<boolean>;
};

// ---------------------------------------------------------------------------
// Error rendering
// ---------------------------------------------------------------------------

function describeApiError(error: ApiError, onDemoDisabled: () => void): string {
  switch (error.kind) {
    case "demo_disabled":
      onDemoDisabled();
      return "";
    case "unavailable":
      return "The API became unavailable during the ceremony. Please try again.";
    case "rate_limited":
      return error.detail;
    case "http_error":
      if (error.status === 410) {
        // ceremony_expired: the challenge timed out or was already used.
        // This is an expected outcome, not a crash — just restart.
        return "This passkey ceremony expired (or was already completed). Start again below.";
      }
      return error.detail;
    default:
      return "Something went wrong. Please try again.";
  }
}

/** `navigator.credentials.create()/get()` rejects with a DOMException. */
function describeWebAuthnError(err: unknown): string {
  const name = err instanceof DOMException ? err.name : undefined;
  switch (name) {
    case "NotAllowedError":
      return "The prompt was dismissed or timed out before finishing. Try again when you're ready.";
    case "InvalidStateError":
      return "This authenticator already has a passkey registered here. Try authenticating instead, or use a different authenticator.";
    case "SecurityError":
      return "The browser blocked this request for security reasons (the site's origin may not match the relying party).";
    case "AbortError":
      return "The request was cancelled. Try again.";
    default:
      return `The browser couldn't complete the ceremony${name ? ` (${name})` : ""}. Try again.`;
  }
}

export default function PasskeysPanel({ scenarioId, onDemoDisabled }: Props) {
  const [capability, setCapability] = useState<Capability>("checking");
  const [unsupportedReason, setUnsupportedReason] = useState<UnsupportedReason | null>(null);
  const [advisory, setAdvisory] = useState<AdvisoryReason | null>(null);

  const [registering, setRegistering] = useState(false);
  const [registerBanner, setRegisterBanner] = useState<string | null>(null);
  const [registerResult, setRegisterResult] = useState<PasskeyEnrolment | null>(null);

  const [authenticating, setAuthenticating] = useState(false);
  const [authBanner, setAuthBanner] = useState<string | null>(null);
  const [authResult, setAuthResult] = useState<PasskeyAuthResult | null>(null);

  useEffect(() => {
    let cancelled = false;

    function markUnsupported(reason: UnsupportedReason) {
      if (!cancelled) {
        setCapability("unsupported");
        setUnsupportedReason(reason);
      }
    }

    async function detect() {
      // Hard block 1: WebAuthn API not available at all.
      if (typeof window === "undefined" || !("PublicKeyCredential" in window)) {
        markUnsupported("no-webauthn");
        return;
      }

      // Hard block 2: Not in a secure context (HTTPS required).
      if (window.isSecureContext === false) {
        markUnsupported("insecure-context");
        return;
      }

      const statics = window.PublicKeyCredential as unknown as PublicKeyCredentialStatics;

      // Soft block: No platform authenticator detected. We can still support
      // passkeys via security keys or cross-device transport (e.g., phone),
      // so we mark as supported but show an advisory.
      if (typeof statics.isUserVerifyingPlatformAuthenticatorAvailable !== "function") {
        if (!cancelled) {
          setCapability("supported");
          setAdvisory("no-platform-authenticator");
        }
        return;
      }

      try {
        const available = await statics.isUserVerifyingPlatformAuthenticatorAvailable();
        if (cancelled) return;
        if (!available) {
          // No platform authenticator, but still support via security keys or phone.
          setCapability("supported");
          setAdvisory("no-platform-authenticator");
          return;
        }
        // Has a platform authenticator — fully supported, no advisory needed.
        setCapability("supported");
      } catch {
        // Error probing the platform authenticator. Assume we can still do
        // WebAuthn via other means (security keys, cross-device).
        if (!cancelled) {
          setCapability("supported");
          setAdvisory("no-platform-authenticator");
        }
      }
    }

    void detect();
    return () => {
      cancelled = true;
    };
  }, []);

  const handleRegister = useCallback(async () => {
    setRegistering(true);
    setRegisterBanner(null);
    setRegisterResult(null);

    try {
      const startResult = await scenarioAction<RegisterStartResponse>(
        scenarioId,
        "register_start",
        {},
      );
      if (!startResult.ok) {
        setRegisterBanner(describeApiError(startResult.error, onDemoDisabled));
        return;
      }

      const options = decodeCreationOptions(startResult.data.publicKey);

      let credential: PublicKeyCredential;
      try {
        const created = await navigator.credentials.create({ publicKey: options });
        if (!created) {
          setRegisterBanner("The browser didn't return a credential. Try again.");
          return;
        }
        credential = created as PublicKeyCredential;
      } catch (err) {
        // Covers user cancellation/timeout (NotAllowedError) and an
        // already-registered authenticator (InvalidStateError) — never leave
        // the button stuck on "Registering…".
        setRegisterBanner(describeWebAuthnError(err));
        return;
      }

      const finishResult = await scenarioAction<PasskeyEnrolment>(
        scenarioId,
        "register_finish",
        encodeAttestation(credential),
      );
      if (!finishResult.ok) {
        setRegisterBanner(describeApiError(finishResult.error, onDemoDisabled));
        return;
      }

      setRegisterResult(finishResult.data);
    } finally {
      setRegistering(false);
    }
  }, [scenarioId, onDemoDisabled]);

  const handleAuthenticate = useCallback(async () => {
    setAuthenticating(true);
    setAuthBanner(null);
    setAuthResult(null);

    try {
      const startResult = await scenarioAction<AuthenticateStartResponse>(
        scenarioId,
        "authenticate_start",
        {},
      );
      if (!startResult.ok) {
        // A 400 invalid_value here typically means no passkey is registered
        // yet — describeApiError surfaces the server's own helpful detail.
        setAuthBanner(describeApiError(startResult.error, onDemoDisabled));
        return;
      }

      const options = decodeRequestOptions(startResult.data.publicKey);

      let credential: PublicKeyCredential;
      try {
        const created = await navigator.credentials.get({ publicKey: options });
        if (!created) {
          setAuthBanner("The browser didn't return a credential. Try again.");
          return;
        }
        credential = created as PublicKeyCredential;
      } catch (err) {
        setAuthBanner(describeWebAuthnError(err));
        return;
      }

      const finishResult = await scenarioAction<PasskeyAuthResult>(
        scenarioId,
        "authenticate_finish",
        encodeAssertion(credential),
      );
      if (!finishResult.ok) {
        setAuthBanner(describeApiError(finishResult.error, onDemoDisabled));
        return;
      }

      // verified: false is a normal outcome, not an error — render it inline.
      setAuthResult(finishResult.data);
    } finally {
      setAuthenticating(false);
    }
  }, [scenarioId, onDemoDisabled]);

  if (capability === "checking") {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Passkeys (WebAuthn)</CardTitle>
          <CardDescription>Register a passkey with this device, then authenticate with it.</CardDescription>
        </CardHeader>
        <CardContent>
          <p className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
            Checking passkey support in this browser…
          </p>
        </CardContent>
      </Card>
    );
  }

  if (capability === "unsupported") {
    return (
      <Card>
        <CardHeader className="flex flex-row items-start justify-between space-y-0">
          <div className="space-y-1.5">
            <CardTitle>Passkeys (WebAuthn)</CardTitle>
            <CardDescription>Register a passkey with this device, then authenticate with it.</CardDescription>
          </div>
          <Badge variant="destructive">Unsupported</Badge>
        </CardHeader>
        <CardContent>
          <Alert role="presentation" className="border-warning/40 bg-warning/10 text-warning-foreground">
            <AlertDescription>
              {unsupportedReason ? UNSUPPORTED_MESSAGES[unsupportedReason] : UNSUPPORTED_MESSAGES["no-webauthn"]}
            </AlertDescription>
          </Alert>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between space-y-0">
        <div className="space-y-1.5">
          <CardTitle>Passkeys (WebAuthn)</CardTitle>
          <CardDescription>Register a passkey with this device, then authenticate with it.</CardDescription>
        </div>
        <Badge variant="secondary">Supported</Badge>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        {advisory && (
          <Alert role="presentation" className="border-warning/40 bg-warning/10 text-warning-foreground">
            <AlertDescription>{ADVISORY_MESSAGES[advisory]}</AlertDescription>
          </Alert>
        )}
        <div className="flex flex-col gap-2">
          {/*
            Deliberately not "this browser's platform authenticator": the panel
            now also runs for devices that have none, where the browser offers a
            security key or a nearby phone instead. Naming the platform one would
            contradict the advisory shown directly above.
          */}
          <p className="text-xs text-muted-foreground">
            Registers a passkey with whichever authenticator this browser offers.
          </p>
          <div>
            <Button type="button" size="sm" onClick={() => void handleRegister()} disabled={registering}>
              {registering && <Loader2 className="animate-spin" aria-hidden="true" />}
              {registering ? "Registering…" : "Register a passkey"}
            </Button>
          </div>
          <div aria-live="polite" role="status">
            {registerBanner && (
              <Alert
                role="presentation"
                className="border-warning/40 bg-warning/10 py-2 text-warning-foreground"
              >
                <AlertDescription>{registerBanner}</AlertDescription>
              </Alert>
            )}
            {registerResult && (
              <Alert
                role="presentation"
                className="border-success/40 bg-success/10 py-2 text-success-foreground"
              >
                <AlertDescription className="flex items-center gap-1.5">
                  <CheckCircle2 className="h-4 w-4 shrink-0" aria-hidden="true" />
                  Passkey registered. This browser now has {registerResult.count}{" "}
                  passkey{registerResult.count === 1 ? "" : "s"} enrolled.
                </AlertDescription>
              </Alert>
            )}
          </div>
        </div>

        <Separator />

        <div className="flex flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            Authenticates using a previously registered passkey.
          </p>
          <div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void handleAuthenticate()}
              disabled={authenticating}
            >
              {authenticating && <Loader2 className="animate-spin" aria-hidden="true" />}
              {authenticating ? "Authenticating…" : "Authenticate with a passkey"}
            </Button>
          </div>
          {/* A passkey ceremony hands control to the browser and the
              authenticator, so nothing on screen changes for seconds at a time.
              Sighted users have the button's "Authenticating…" label; without a
              live region a screen-reader user gets silence, then a result they
              did not know was coming. */}
          <div aria-live="polite" role="status">
            {authenticating && (
              <p className="flex items-center gap-2 text-xs text-muted-foreground">
                <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                Waiting for your authenticator…
              </p>
            )}
            {authBanner && (
              <Alert
                role="presentation"
                className="border-warning/40 bg-warning/10 py-2 text-warning-foreground"
              >
                <AlertDescription>{authBanner}</AlertDescription>
              </Alert>
            )}
            {authResult && (
              <Alert
                role="presentation"
                className={cn(
                  "py-2",
                  authResult.verified
                    ? "border-success/40 bg-success/10 text-success-foreground"
                    : "border-border bg-muted/40 text-muted-foreground",
                )}
              >
                <AlertDescription className="flex flex-col gap-1">
                  <span className="flex items-center gap-1.5">
                    {authResult.verified ? (
                      <CheckCircle2 className="h-4 w-4 shrink-0" aria-hidden="true" />
                    ) : (
                      <Circle className="h-4 w-4 shrink-0" aria-hidden="true" />
                    )}
                    {authResult.detail}
                  </span>
                  {authResult.counter !== null && (
                    <span className="text-muted-foreground">
                      Signature counter: {authResult.counter}. A counter that fails to
                      advance between authentications is how cloned authenticators are
                      detected.
                    </span>
                  )}
                </AlertDescription>
              </Alert>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
