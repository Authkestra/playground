//! The TypeScript client emitted alongside a generated project.
//!
//! Hand-written rather than generated from the OpenAPI document, and
//! deliberately so: the hard part of a passkey integration is not the request
//! shapes, it is the base64url conversion around `navigator.credentials`,
//! which no schema describes and every code generator gets wrong by omission.
//!
//! Emitted only when the visitor asked for it. Someone who wants a Rust
//! service should not find a TypeScript file in their download.

use super::Plan;

/// The client's path inside the generated project.
pub const CLIENT_PATH: &str = "client/authkestra.ts";

pub fn client_ts(plan: &Plan) -> String {
    let mut out = String::from(
        r#"// A typed client for this project's authentication endpoints.
//
// Hand-written, not generated from the OpenAPI document. A schema describes
// what the wire looks like; it cannot describe the base64url conversion that
// `navigator.credentials` requires on both sides, which is where most passkey
// integrations break. That conversion is the point of this file.
//
// No dependencies, no build step. Copy it, or import it as-is.

/** Where this project is served from. Change it, or pass a base to each call. */
const DEFAULT_BASE = "";

export class AuthError extends Error {
  constructor(
    readonly status: number,
    readonly detail: string,
  ) {
    super(detail);
    this.name = "AuthError";
  }
}

async function post<T>(path: string, body: unknown, base = DEFAULT_BASE): Promise<T> {
  const res = await fetch(`${base}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    // Sessions are cookie-based, so cross-origin callers need credentials.
    credentials: "include",
    body: JSON.stringify(body),
  });

  const payload = await res.json().catch(() => ({}) as Record<string, unknown>);
  if (!res.ok) {
    const detail =
      typeof payload.detail === "string"
        ? payload.detail
        : typeof payload.error === "string"
          ? payload.error
          : res.statusText;
    throw new AuthError(res.status, detail);
  }
  return payload as T;
}
"#,
    );

    if plan.is_active("passkeys") {
        out.push_str(PASSKEY_CLIENT);
    }
    if plan.is_active("totp") {
        out.push_str(TOTP_CLIENT);
    }
    out
}

const PASSKEY_CLIENT: &str = r#"
// ---------------------------------------------------------------- base64url
//
// WebAuthn speaks ArrayBuffers; JSON does not. The server sends base64url and
// expects base64url back. Plain base64 is *not* interchangeable: it uses `+`
// and `/`, which are not URL-safe, and it pads with `=`.
//
// Getting this wrong does not fail loudly. The ceremony fails inside the
// browser with a deliberately vague error, or the server rejects a signature
// that was actually valid.

function fromBase64Url(value: string): ArrayBuffer {
  const padded = value.replace(/-/g, "+").replace(/_/g, "/");
  const binary = atob(padded.padEnd(padded.length + ((4 - (padded.length % 4)) % 4), "="));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

function toBase64Url(value: ArrayBuffer): string {
  const bytes = new Uint8Array(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** What the server returns to start either ceremony. */
interface Challenge {
  ceremony_id: string;
  publicKey: Record<string, unknown>;
}

/** Decode every field WebAuthn needs as a buffer, and leave the rest alone. */
function decodeCreationOptions(
  publicKey: Record<string, unknown>,
): PublicKeyCredentialCreationOptions {
  const options = { ...publicKey } as Record<string, unknown>;
  options.challenge = fromBase64Url(publicKey.challenge as string);

  const user = { ...(publicKey.user as Record<string, unknown>) };
  user.id = fromBase64Url(user.id as string);
  options.user = user;

  // Present when re-enrolling, so the authenticator can refuse a duplicate.
  const exclude = publicKey.excludeCredentials as Array<Record<string, unknown>> | undefined;
  if (exclude) {
    options.excludeCredentials = exclude.map((c) => ({
      ...c,
      id: fromBase64Url(c.id as string),
    }));
  }
  return options as unknown as PublicKeyCredentialCreationOptions;
}

function decodeRequestOptions(
  publicKey: Record<string, unknown>,
): PublicKeyCredentialRequestOptions {
  const options = { ...publicKey } as Record<string, unknown>;
  options.challenge = fromBase64Url(publicKey.challenge as string);

  const allow = publicKey.allowCredentials as Array<Record<string, unknown>> | undefined;
  if (allow) {
    options.allowCredentials = allow.map((c) => ({
      ...c,
      id: fromBase64Url(c.id as string),
    }));
  }
  return options as unknown as PublicKeyCredentialRequestOptions;
}

/**
 * Encode an assertion for the server.
 *
 * `clientDataJSON` is spelled with `JSON` fully capitalised. That is a genuine
 * quirk of the WebAuthn spec and the one field a blanket camelCase rule gets
 * wrong — `clientDataJson` is what such a rule produces, and no server that
 * follows the spec will accept it. Do not "tidy" this.
 */
function encodeAssertion(credential: PublicKeyCredential) {
  const response = credential.response as AuthenticatorAssertionResponse;
  return {
    id: credential.id,
    response: {
      clientDataJSON: toBase64Url(response.clientDataJSON),
      authenticatorData: toBase64Url(response.authenticatorData),
      signature: toBase64Url(response.signature),
      userHandle: response.userHandle ? toBase64Url(response.userHandle) : null,
    },
  };
}

function encodeAttestation(credential: PublicKeyCredential) {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: toBase64Url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: toBase64Url(response.clientDataJSON),
      attestationObject: toBase64Url(response.attestationObject),
    },
  };
}

// ------------------------------------------------------------------ passkeys

/** Enrol a new passkey for `username`. Prompts the authenticator. */
export async function enrolPasskey(username: string, base?: string): Promise<void> {
  const challenge = await post<Challenge>("/auth/passkey/register/start", { username }, base);

  const credential = (await navigator.credentials.create({
    publicKey: decodeCreationOptions(challenge.publicKey),
  })) as PublicKeyCredential | null;

  if (!credential) throw new AuthError(0, "the authenticator returned nothing");

  await post<{ enrolled: boolean }>(
    "/auth/passkey/register/finish",
    { ceremony_id: challenge.ceremony_id, credential: encodeAttestation(credential) },
    base,
  );
}

/** Sign in with an enrolled passkey. Opens a session on success. */
export async function signInWithPasskey(username: string, base?: string): Promise<void> {
  const challenge = await post<Challenge>("/auth/passkey/login/start", { username }, base);

  const credential = (await navigator.credentials.get({
    publicKey: decodeRequestOptions(challenge.publicKey),
  })) as PublicKeyCredential | null;

  if (!credential) throw new AuthError(0, "the authenticator returned nothing");

  await post<{ verified: boolean }>(
    "/auth/passkey/login/finish",
    { ceremony_id: challenge.ceremony_id, credential: encodeAssertion(credential) },
    base,
  );
}
"#;

const TOTP_CLIENT: &str = r#"
// ---------------------------------------------------------------------- TOTP

export interface TotpEnrolment {
  /** Base32, for manual entry. Treat it as a credential: never log it. */
  secret: string;
  /** `otpauth://` URI. Render it as a QR code. */
  uri: string;
}

/**
 * Enrol an authenticator app.
 *
 * Enrolling again replaces any previous authenticator, so the secret returned
 * here is always the live one. Show the URI as a QR code and the secret only
 * as the manual fallback.
 */
export async function enrolTotp(username: string, base?: string): Promise<TotpEnrolment> {
  return post<TotpEnrolment>("/auth/totp/enroll", { username }, base);
}

/** Check a six-digit code. Returns false for a wrong or replayed code. */
export async function verifyTotp(
  username: string,
  code: string,
  base?: string,
): Promise<boolean> {
  try {
    const result = await post<{ verified: boolean }>(
      "/auth/totp/verify",
      { username, code },
      base,
    );
    return result.verified;
  } catch (error) {
    // A wrong code answers 401, which is an ordinary outcome here rather than
    // something to propagate as an exception.
    if (error instanceof AuthError && error.status === 401) return false;
    throw error;
  }
}
"#;
