/**
 * Reading and verifying a JWT in the visitor's own browser (#76).
 *
 * ## Why this exists at all
 *
 * The resource scenario is the one ceremony in the playground with no
 * independent party in it. TOTP convinces because the code comes from an
 * authenticator we do not control; passkeys convince because the assertion
 * comes from a secure enclave we cannot fake. The resource scenario mints the
 * token, validates it, and narrates the outcome — all in the same process. A
 * visitor reading "401 — issued for another service" has no way to tell that
 * apart from a hardcoded string.
 *
 * The one independent party available is the visitor's own browser, so this
 * module recruits it. Everything here runs client-side against bytes the
 * visitor can see: the token in the textarea, and a key set they fetched
 * themselves from a URL the panel prints in full.
 *
 * ## The rule that makes it worth more than the server's answer
 *
 * A badge drawn by our page is worth no more than a badge drawn by our API
 * unless the visitor can tell the difference — and no pixel we render can
 * establish that, because we render the pixels. The evidence has to come from
 * instrumentation we do not control: the browser's own network panel, or the
 * network being switched off entirely.
 *
 * So [`verifyTokenSignature`] makes **zero** network requests. Not "normally
 * none" — none. The key set is a parameter, fetched earlier by an explicit,
 * separate step ([`fetchJwks`]), which is what makes the silence at verdict
 * time meaningful. `jwt.test.ts` fails if `fetch` is so much as touched during
 * verification, so this cannot quietly regress into a round trip later.
 *
 * ## Ed25519 in WebCrypto is recent
 *
 * `crypto.subtle` grew Ed25519 late (Safari 17, Firefox 130, Chrome 137), so
 * a browser that cannot do it at all is a real visitor, not a hypothetical.
 * Every entry point here reports that as its own outcome — see
 * [`Ed25519Support`] and the `unsupported` verdict — rather than falling back
 * to a shipped JavaScript implementation. A verdict computed by a signature
 * library we served would be our answer again, wearing the browser's clothes,
 * which is the exact thing this module exists to stop doing.
 */

/** The decoded header segment of a JWT. */
export interface JwtHeader {
  alg?: string;
  kid?: string;
  typ?: string;
  [claim: string]: unknown;
}

/** The decoded payload segment of a JWT: the claims themselves. */
export interface JwtClaims {
  iss?: string;
  aud?: string | string[];
  sub?: string;
  exp?: number;
  iat?: number;
  nbf?: number;
  [claim: string]: unknown;
}

/**
 * One key from a JWKS, as published.
 *
 * Every field is optional because this is somebody else's JSON: it arrives
 * over the network from a URL the visitor can edit, and a key set missing the
 * field we need is a case to report, not to crash on.
 */
export interface Jwk {
  kty?: string;
  crv?: string;
  alg?: string;
  kid?: string;
  x?: string;
  [field: string]: unknown;
}

/**
 * base64url to bytes.
 *
 * One decoder for both jobs — reading a JSON segment and reading the raw
 * signature — because they are the same operation and the signature is not
 * text. Decoding straight to a string via `atob` would also quietly mangle any
 * non-ASCII claim, since `atob` yields latin-1 code units, not UTF-8.
 */
function base64UrlToBytes(segment: string): ArrayBuffer | null {
  try {
    const binary = atob(segment.replace(/-/g, "+").replace(/_/g, "/"));
    // An `ArrayBuffer` rather than a bare `Uint8Array`, because both consumers
    // — `TextDecoder` and `SubtleCrypto.verify` — want a `BufferSource`, and a
    // typed array's buffer is only *maybe* one as far as the type system is
    // concerned.
    const buffer = new ArrayBuffer(binary.length);
    const bytes = new Uint8Array(buffer);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return buffer;
  } catch {
    return null;
  }
}

/** One decoded JSON segment of a JWT, or null if it is not readable as one. */
function decodeSegment(token: string, index: 0 | 1): Record<string, unknown> | null {
  const segment = token.trim().split(".")[index];
  if (!segment) return null;
  const bytes = base64UrlToBytes(segment);
  if (!bytes) return null;
  try {
    const value: unknown = JSON.parse(new TextDecoder().decode(bytes));
    return typeof value === "object" && value !== null && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

/** The header segment, decoded. Shown so the `kid` is not hearsay. */
export function decodeHeader(token: string): JwtHeader | null {
  return decodeSegment(token, 0);
}

/**
 * The payload segment, decoded. Shown so the *reason* for a verdict is not
 * hearsay either.
 *
 * A visitor who can read `aud`, `iss`, `exp` and `sub` out of the token
 * themselves no longer has to take "issued for another service" on our word —
 * four of the six forgeries stop being claims and become something they can
 * see. That is the cheapest honesty available in the whole playground, and it
 * is one function beside one that already existed.
 */
export function decodeClaims(token: string): JwtClaims | null {
  return decodeSegment(token, 1);
}

/** `aud` is a string or an array of them; render both the same way. */
export function formatAudience(aud: JwtClaims["aud"]): string | null {
  if (typeof aud === "string") return aud;
  if (Array.isArray(aud)) return aud.join(", ");
  return null;
}

/**
 * `exp`, against the visitor's own clock rather than ours.
 *
 * Deliberately the *browser's* clock: "expired" is the one forgery whose
 * reason a visitor can check with a wall clock, and checking it against a
 * timestamp the API also sent would put us back on both sides of the question.
 */
export function describeExpiry(
  exp: unknown,
  now: number = Date.now(),
): { expired: boolean; text: string } | null {
  if (typeof exp !== "number" || !Number.isFinite(exp)) return null;

  const deltaS = Math.round(exp * 1000 - now) / 1000;
  const magnitude = Math.abs(deltaS);
  const amount = magnitude < 90 ? `${Math.round(magnitude)}s` : `${Math.round(magnitude / 60)}m`;
  const at = new Date(exp * 1000).toLocaleTimeString();

  return deltaS < 0
    ? { expired: true, text: `expired ${amount} ago, at ${at} by your clock` }
    : { expired: false, text: `expires in ${amount}, at ${at} by your clock` };
}

// ------------------------------------------------------------------ key sets

/**
 * Fetches a key set. **The only function here that touches the network**, and
 * it is a step the visitor takes on purpose.
 *
 * Splitting this out is the whole design. If verification fetched its own
 * keys, a silent network panel would prove nothing and an offline browser
 * would simply fail — there would be no moment at which the page visibly
 * answers with no help from us.
 */
export async function fetchJwks(url: string): Promise<Jwk[]> {
  const response = await fetch(url, { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`${url} answered ${response.status}`);
  }
  const body: unknown = await response.json();
  const keys = (body as { keys?: unknown })?.keys;
  if (!Array.isArray(keys)) {
    throw new Error(`${url} returned no \`keys\` array`);
  }
  return keys as Jwk[];
}

// -------------------------------------------------------------- verification

/**
 * What the browser concluded, and why.
 *
 * Every case that is *not* a signature decision is its own variant rather than
 * a boolean plus a message, because the interesting half of this scenario is
 * the difference between "the signature is wrong" and "there is no key to
 * check it against" — collapsing them would throw away exactly what the
 * forgeries are demonstrating.
 */
export type LocalVerdict =
  /** The signature checks out against a key from the fetched set. */
  | { kind: "verified"; kid: string }
  /** A key with that `kid` was found, and it did not sign this token. */
  | { kind: "bad_signature"; kid: string }
  /** The `kid` is not in the key set that was fetched. */
  | { kind: "unknown_kid"; kid: string }
  /** No `kid` header, so no key can be chosen without guessing. */
  | { kind: "missing_kid" }
  /** Not three readable segments; nothing to verify. */
  | { kind: "malformed"; reason: string }
  /** A real token, signed with something this page cannot check. */
  | { kind: "unsupported_alg"; alg: string }
  /** This browser's WebCrypto cannot do Ed25519 at all. */
  | { kind: "unsupported"; reason: string };

/**
 * A public key that exists only to ask the browser a question.
 *
 * Feature-detecting Ed25519 by looking for a method name does not work — every
 * engine has `importKey`, and the ones without Ed25519 reject the *algorithm*.
 * So the probe is a real import of a real key (an Ed25519 public key generated
 * for this purpose and otherwise unused; its private half was discarded and
 * nothing is ever verified against it).
 */
const PROBE_KEY_X = "C-wD1JWigTwQCpcB2heDf55rQb-hdakJUTbG-TQj7BU";

/** Whether this browser can verify Ed25519, and if not, what to say about it. */
export type Ed25519Support =
  | { supported: true }
  | { supported: false; reason: string };

/**
 * Asks the browser, once, whether it can do Ed25519 — so the panel can say so
 * up front instead of letting a visitor press verify and get an error.
 *
 * `subtle` is a parameter so the unsupported path can be *tested* rather than
 * asserted: we cannot make Node forget Ed25519, and an honest error path that
 * nobody has ever run is not an honest error path.
 */
export async function checkEd25519Support(
  subtle: SubtleCrypto | null = globalThis.crypto?.subtle ?? null,
): Promise<Ed25519Support> {
  if (!subtle) {
    return {
      supported: false,
      // The likeliest cause by far, and the one the visitor can act on.
      reason:
        "This page has no WebCrypto. That usually means it is not being served over HTTPS, " +
        "which `crypto.subtle` requires.",
    };
  }
  try {
    await importEd25519(subtle, PROBE_KEY_X);
    return { supported: true };
  } catch {
    return {
      supported: false,
      reason:
        "This browser's WebCrypto has no Ed25519. It arrived in Safari 17, Firefox 130 and " +
        "Chrome 137 — an older one cannot check this signature, and we would rather say so " +
        "than show you a verdict we did not compute.",
    };
  }
}

/**
 * Imports a JWKS `x` as an Ed25519 verification key.
 *
 * `alg` is deliberately dropped from the JWK before import. WebCrypto's Secure
 * Curves rules reject an Ed25519 JWK whose `alg` is anything but `EdDSA`, and
 * a deployment is free to publish `Ed25519` there instead; since `alg` is
 * optional on import and we have already decided what curve we are using, the
 * safest thing to hand the browser is the key material and nothing else.
 */
function importEd25519(subtle: SubtleCrypto, x: string): Promise<CryptoKey> {
  return subtle.importKey("jwk", { kty: "OKP", crv: "Ed25519", x }, { name: "Ed25519" }, false, [
    "verify",
  ]);
}

/**
 * Verifies a token's signature **in this browser, with no network access**.
 *
 * `keys` is a parameter, not something this fetches: see the module docs. The
 * consequence is the demonstration — a visitor can switch their network off,
 * press verify, and still get an answer, which is the same claim as "a
 * resource server validates without calling the issuer" made in one gesture
 * and with no expertise required.
 *
 * `subtle` is injectable for the same reason as in [`checkEd25519Support`].
 */
export async function verifyTokenSignature(
  token: string,
  keys: Jwk[],
  subtle: SubtleCrypto | null = globalThis.crypto?.subtle ?? null,
): Promise<LocalVerdict> {
  const parts = token.trim().split(".");
  if (parts.length !== 3) {
    return { kind: "malformed", reason: `a JWT has three segments; this has ${parts.length}` };
  }

  const header = decodeHeader(token);
  if (!header) return { kind: "malformed", reason: "the header segment does not decode as JSON" };
  if (!decodeClaims(token)) {
    return { kind: "malformed", reason: "the payload segment does not decode as JSON" };
  }

  // Checked before the key lookup so the message names the real problem: a
  // token we cannot check is not a token that failed.
  if (header.alg && header.alg !== "EdDSA" && header.alg !== "Ed25519") {
    return { kind: "unsupported_alg", alg: header.alg };
  }

  // The same strict policy the resource server applies, and for the same
  // reason: falling back to "try every published key" is how a retired key
  // keeps working for as long as it stays published.
  if (typeof header.kid !== "string" || header.kid === "") return { kind: "missing_kid" };
  const kid = header.kid;

  const key = keys.find((k) => k.kid === kid);
  if (!key) return { kind: "unknown_kid", kid };
  if (typeof key.x !== "string" || (key.crv !== undefined && key.crv !== "Ed25519")) {
    return {
      kind: "unsupported",
      reason: `the published key \`${kid}\` is not an Ed25519 public key this page can import`,
    };
  }

  const signature = base64UrlToBytes(parts[2]);
  if (!signature) return { kind: "malformed", reason: "the signature segment is not base64url" };

  if (!subtle) {
    const support = await checkEd25519Support(subtle);
    return { kind: "unsupported", reason: support.supported ? "no WebCrypto" : support.reason };
  }

  let publicKey: CryptoKey;
  try {
    publicKey = await importEd25519(subtle, key.x);
  } catch {
    const support = await checkEd25519Support(subtle);
    return {
      kind: "unsupported",
      reason: support.supported
        ? `the published key \`${kid}\` was rejected as an Ed25519 public key`
        : support.reason,
    };
  }

  // What Ed25519 actually signs: the first two segments and the dot between
  // them, exactly as they arrived. Re-encoding the decoded JSON would change
  // the bytes and fail a perfectly good token.
  const signed = new TextEncoder().encode(`${parts[0]}.${parts[1]}`);

  const ok = await subtle.verify({ name: "Ed25519" }, publicKey, signature, signed);
  return ok ? { kind: "verified", kid } : { kind: "bad_signature", kid };
}

/** One line per verdict, in the visitor's terms rather than the algorithm's. */
export function describeVerdict(verdict: LocalVerdict): string {
  switch (verdict.kind) {
    case "verified":
      return `Signature checks out against the published key \`${verdict.kid}\`.`;
    case "bad_signature":
      return `The key \`${verdict.kid}\` is published, and it did not sign this token.`;
    case "unknown_kid":
      return `No key called \`${verdict.kid}\` in the key set you fetched.`;
    case "missing_kid":
      return "No `kid` in the header, so there is no key to check this against.";
    case "malformed":
      return `Not a readable token: ${verdict.reason}.`;
    case "unsupported_alg":
      return `Signed with \`${verdict.alg}\`, which this page does not check.`;
    case "unsupported":
      return verdict.reason;
  }
}
