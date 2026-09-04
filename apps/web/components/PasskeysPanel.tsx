"use client";

import { useCallback, useEffect, useState } from "react";
import type { PasskeyAuthResult, PasskeyEnrolment } from "@playground/api-types";
import { scenarioAction, type ApiError } from "@/lib/api";

interface Props {
  scenarioId: string;
  /** Bubble up: the demo-wide kill switch flipped mid-ceremony. */
  onDemoDisabled: () => void;
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
function base64UrlToBuffer(value: string): Uint8Array {
  const base64 = value.replace(/-/g, "+").replace(/_/g, "/");
  const padLength = (4 - (base64.length % 4)) % 4;
  const padded = base64 + "=".repeat(padLength);
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

/** ArrayBuffer -> base64url (see base64UrlToBuffer for the alphabet note). */
function bufferToBase64Url(buffer: ArrayBuffer): string {
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
type UnsupportedReason = "no-webauthn" | "insecure-context" | "no-platform-authenticator";

const UNSUPPORTED_MESSAGES: Record<UnsupportedReason, string> = {
  "no-webauthn":
    "This browser doesn't support passkeys (the WebAuthn API isn't available). " +
    "Try the \"Authenticator app (TOTP)\" scenario instead — it works everywhere.",
  "insecure-context":
    "Passkeys require a secure context (HTTPS), and this page isn't loaded over one. " +
    "Try the \"Authenticator app (TOTP)\" scenario instead.",
  "no-platform-authenticator":
    "No platform authenticator (like Touch ID, Windows Hello, or a fingerprint sensor) " +
    "was detected on this device. Try the \"Authenticator app (TOTP)\" scenario instead.",
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
      if (typeof window === "undefined" || !("PublicKeyCredential" in window)) {
        markUnsupported("no-webauthn");
        return;
      }

      if (window.isSecureContext === false) {
        markUnsupported("insecure-context");
        return;
      }

      const statics = window.PublicKeyCredential as unknown as PublicKeyCredentialStatics;
      if (typeof statics.isUserVerifyingPlatformAuthenticatorAvailable !== "function") {
        // The interface exists but not the capability check — too old/partial
        // an implementation to trust with a real ceremony.
        markUnsupported("no-webauthn");
        return;
      }

      try {
        const available = await statics.isUserVerifyingPlatformAuthenticatorAvailable();
        if (cancelled) return;
        if (!available) {
          markUnsupported("no-platform-authenticator");
          return;
        }
        setCapability("supported");
      } catch {
        markUnsupported("no-platform-authenticator");
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
    return <p className="text-xs text-slate-400">Checking passkey support in this browser…</p>;
  }

  if (capability === "unsupported") {
    return (
      <div className="rounded-md border border-slate-200 bg-slate-50 p-3">
        <p className="text-xs text-slate-600">
          {unsupportedReason ? UNSUPPORTED_MESSAGES[unsupportedReason] : UNSUPPORTED_MESSAGES["no-webauthn"]}
        </p>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-2">
        <p className="text-xs text-slate-500">
          Registers a passkey with this browser&apos;s platform authenticator.
        </p>
        <div>
          <button
            type="button"
            onClick={() => void handleRegister()}
            disabled={registering}
            className="rounded-md border border-slate-300 px-2.5 py-1 text-xs font-medium text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {registering ? "Registering…" : "Register a passkey"}
          </button>
        </div>
        {registerBanner && (
          <p className="flex items-center gap-1.5 text-xs text-amber-600">
            {registerBanner}
          </p>
        )}
        {registerResult && (
          <p className="flex items-center gap-1.5 text-xs text-emerald-600">
            <span aria-hidden="true">✓</span>
            Passkey registered. This browser now has {registerResult.count}{" "}
            passkey{registerResult.count === 1 ? "" : "s"} enrolled.
          </p>
        )}
      </div>

      <div className="flex flex-col gap-2 border-t border-slate-100 pt-4">
        <p className="text-xs text-slate-500">
          Authenticates using a previously registered passkey.
        </p>
        <div>
          <button
            type="button"
            onClick={() => void handleAuthenticate()}
            disabled={authenticating}
            className="rounded-md border border-slate-300 px-2.5 py-1 text-xs font-medium text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {authenticating ? "Authenticating…" : "Authenticate with a passkey"}
          </button>
        </div>
        {authBanner && (
          <p className="flex items-center gap-1.5 text-xs text-amber-600">{authBanner}</p>
        )}
        {authResult && (
          <div
            className={`flex flex-col gap-1 text-xs ${
              authResult.verified ? "text-emerald-600" : "text-slate-600"
            }`}
          >
            <p className="flex items-center gap-1.5">
              <span aria-hidden="true">{authResult.verified ? "✓" : "•"}</span>
              {authResult.detail}
            </p>
            {authResult.counter !== null && (
              <p className="text-slate-400">
                Signature counter: {authResult.counter}. A counter that fails to
                advance between authentications is how cloned authenticators are
                detected.
              </p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
