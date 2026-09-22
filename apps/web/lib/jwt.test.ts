import { describe, it, expect, afterEach, vi } from "vitest";
import {
  checkEd25519Support,
  decodeClaims,
  decodeHeader,
  describeExpiry,
  describeVerdict,
  fetchJwks,
  formatAudience,
  verifyTokenSignature,
  type Jwk,
  type LocalVerdict,
} from "./jwt";

/**
 * A real Ed25519 key pair, minted per test run, plus a real token signed with
 * it. Nothing here is a fixture of the expected answer: the tests sign tokens
 * and then ask the same code the browser runs whether the signature holds, so
 * a bug in verification cannot be papered over by a canned string.
 */
async function keypair(): Promise<CryptoKeyPair> {
  return (await crypto.subtle.generateKey({ name: "Ed25519" }, true, [
    "sign",
    "verify",
  ])) as CryptoKeyPair;
}

/** The JWKS entry a deployment would publish for a key, in the live shape. */
async function jwkFor(pair: CryptoKeyPair, kid: string): Promise<Jwk> {
  const jwk = (await crypto.subtle.exportKey("jwk", pair.publicKey)) as JsonWebKey;
  // `alg: "EdDSA"` mirrors what /.well-known/jwks.json actually publishes, and
  // is exactly the field the importer has to drop rather than pass through.
  return { kty: "OKP", crv: "Ed25519", alg: "EdDSA", kid, x: jwk.x };
}

function b64url(value: object): string {
  return Buffer.from(JSON.stringify(value)).toString("base64url");
}

/** Signs a header and payload into a real three-segment token. */
async function sign(
  pair: CryptoKeyPair,
  header: object,
  claims: object = { sub: "demo" },
): Promise<string> {
  const signed = `${b64url(header)}.${b64url(claims)}`;
  const signature = await crypto.subtle.sign(
    { name: "Ed25519" },
    pair.privateKey,
    new TextEncoder().encode(signed),
  );
  return `${signed}.${Buffer.from(signature).toString("base64url")}`;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("decodeHeader", () => {
  // The point is that the `kid` shown next to the token is read out of the
  // token, not taken from a field the server also sent.
  it("reads the kid out of a real token header", () => {
    const header = Buffer.from(JSON.stringify({ alg: "EdDSA", kid: "key-1" })).toString(
      "base64url",
    );
    expect(decodeHeader(`${header}.payload.signature`)).toEqual({ alg: "EdDSA", kid: "key-1" });
  });

  it("returns null for anything it cannot read, rather than throwing", () => {
    for (const junk of ["", "not-base64!!", "....", "hello"]) {
      expect(() => decodeHeader(junk)).not.toThrow();
      expect(decodeHeader(junk)).toBeNull();
    }
  });
});

describe("decodeClaims", () => {
  // Four of the six forgeries are self-evident from these four claims, which
  // is the whole of tier 1.
  it("reads the claims a rejection is usually about", () => {
    const token = `${b64url({ alg: "EdDSA" })}.${b64url({
      iss: "https://issuer.example.com",
      aud: "somebody-elses-api",
      sub: "demo-user",
      exp: 1_700_000_000,
    })}.sig`;

    expect(decodeClaims(token)).toEqual({
      iss: "https://issuer.example.com",
      aud: "somebody-elses-api",
      sub: "demo-user",
      exp: 1_700_000_000,
    });
  });

  it("survives a claim that is not ASCII", () => {
    const payload = Buffer.from(JSON.stringify({ sub: "Ünïcøde ✓" })).toString("base64url");
    expect(decodeClaims(`h.${payload}.s`)?.sub).toBe("Ünïcøde ✓");
  });

  it("returns null rather than throwing on a payload that is not an object", () => {
    expect(decodeClaims(`h.${Buffer.from('"just a string"').toString("base64url")}.s`)).toBeNull();
    expect(decodeClaims("h..s")).toBeNull();
  });
});

describe("formatAudience", () => {
  it("renders both shapes `aud` is allowed to take", () => {
    expect(formatAudience("playground-api")).toBe("playground-api");
    expect(formatAudience(["playground-api", "other"])).toBe("playground-api, other");
    expect(formatAudience(undefined)).toBeNull();
  });
});

describe("describeExpiry", () => {
  const now = Date.UTC(2026, 0, 1, 12, 0, 0);

  it("judges `exp` against the clock it was given, in both directions", () => {
    expect(describeExpiry(now / 1000 - 120, now)).toMatchObject({ expired: true });
    expect(describeExpiry(now / 1000 + 45, now)).toMatchObject({ expired: false });
    expect(describeExpiry(now / 1000 - 120, now)?.text).toContain("2m ago");
    expect(describeExpiry(now / 1000 + 45, now)?.text).toContain("45s");
  });

  it("returns null for a token that carries no usable `exp`", () => {
    expect(describeExpiry(undefined, now)).toBeNull();
    expect(describeExpiry("soon", now)).toBeNull();
  });
});

describe("verifyTokenSignature", () => {
  it("accepts a token the published key really signed", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "live" });
    expect(await verifyTokenSignature(token, [await jwkFor(pair, "live")])).toEqual({
      kind: "verified",
      kid: "live",
    });
  });

  // The forgery that claims a real `kid` it did not sign with — the one case
  // where only cryptography can tell the difference.
  it("refuses a token signed by a different key under a published kid", async () => {
    const real = await keypair();
    const impostor = await keypair();
    const token = await sign(impostor, { alg: "EdDSA", kid: "live" });
    expect(await verifyTokenSignature(token, [await jwkFor(real, "live")])).toEqual({
      kind: "bad_signature",
      kid: "live",
    });
  });

  it("refuses a token whose signature was tampered with after signing", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "live" });
    const [header, payload, signature] = token.split(".");
    const flipped = `${signature[0] === "A" ? "B" : "A"}${signature.slice(1)}`;
    expect(await verifyTokenSignature(`${header}.${payload}.${flipped}`, [
      await jwkFor(pair, "live"),
    ])).toMatchObject({ kind: "bad_signature" });
  });

  it("refuses a kid that is not in the key set it was handed", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "never-published" });
    expect(await verifyTokenSignature(token, [await jwkFor(pair, "live")])).toEqual({
      kind: "unknown_kid",
      kid: "never-published",
    });
  });

  it("refuses to guess when there is no kid at all", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA" });
    // Even though the one published key would have verified it. Trying every
    // key is how a retired key keeps working for as long as it stays listed.
    expect(await verifyTokenSignature(token, [await jwkFor(pair, "live")])).toEqual({
      kind: "missing_kid",
    });
  });

  it("reports an unreadable token as malformed rather than as a bad signature", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "live" });
    const keys = [await jwkFor(pair, "live")];

    for (const junk of ["", "not-a-token", `${token}.extra`, "a.b.c"]) {
      expect(await verifyTokenSignature(junk, keys)).toMatchObject({ kind: "malformed" });
    }
  });

  it("says so, rather than failing, when the token is signed with another algorithm", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "RS256", kid: "live" });
    expect(await verifyTokenSignature(token, [await jwkFor(pair, "live")])).toEqual({
      kind: "unsupported_alg",
      alg: "RS256",
    });
  });

  it("reports an unusable published key instead of calling the token forged", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "live" });
    const rsa: Jwk = { kty: "RSA", kid: "live", n: "…", e: "AQAB" };
    expect(await verifyTokenSignature(token, [rsa])).toMatchObject({ kind: "unsupported" });
  });

  /**
   * The honest-failure path for a browser without Ed25519 (anything older than
   * Safari 17 / Firefox 130 / Chrome 137). It cannot be reached by feature
   * detection here — Node has Ed25519 — so the subtle implementation is
   * substituted, which is the reason it is a parameter at all. An error path
   * nobody has run is not an error path.
   */
  it("says the browser cannot check it, rather than showing a verdict it did not compute", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "live" });
    const refuses = {
      importKey: () => Promise.reject(new DOMException("Unrecognized name.", "NotSupportedError")),
    } as unknown as SubtleCrypto;

    const verdict = await verifyTokenSignature(token, [await jwkFor(pair, "live")], refuses);
    expect(verdict.kind).toBe("unsupported");
    expect(describeVerdict(verdict)).toContain("Ed25519");
  });

  it("reports missing WebCrypto instead of throwing on an insecure page", async () => {
    const pair = await keypair();
    const token = await sign(pair, { alg: "EdDSA", kid: "live" });
    const verdict = await verifyTokenSignature(token, [await jwkFor(pair, "live")], null);
    expect(verdict).toMatchObject({ kind: "unsupported" });
    expect(describeVerdict(verdict)).toContain("HTTPS");
  });

  /**
   * The load-bearing test of the whole feature.
   *
   * A browser-computed verdict is only worth more than a server-computed one
   * if the visitor can tell the difference, and the only way they can is that
   * verification is observably silent — network panel quiet, or the network
   * switched off entirely. That guarantee lives in code, so it needs a test
   * that breaks the moment someone reaches for a fetch in here.
   */
  it("makes no network request of any kind while verifying", async () => {
    const pair = await keypair();
    const keys = [await jwkFor(pair, "live")];
    const tokens = [
      await sign(pair, { alg: "EdDSA", kid: "live" }),
      await sign(pair, { alg: "EdDSA", kid: "unpublished" }),
      await sign(pair, { alg: "EdDSA" }),
      "not-a-token",
    ];

    const fetchSpy = vi.fn(() => {
      throw new Error("verification reached for the network");
    });
    vi.stubGlobal("fetch", fetchSpy);
    // The other two ways a browser can make a request, in case a future
    // refactor routes around `fetch`.
    vi.stubGlobal(
      "XMLHttpRequest",
      class {
        constructor() {
          throw new Error("verification reached for the network");
        }
      },
    );
    const beacon = vi.fn(() => true);
    vi.stubGlobal("navigator", { sendBeacon: beacon });

    const verdicts: LocalVerdict[] = [];
    for (const token of tokens) verdicts.push(await verifyTokenSignature(token, keys));

    expect(fetchSpy).not.toHaveBeenCalled();
    expect(beacon).not.toHaveBeenCalled();
    // And it still answered, which is what a visitor sees when they pull the
    // plug and press verify anyway.
    expect(verdicts.map((v) => v.kind)).toEqual([
      "verified",
      "unknown_kid",
      "missing_kid",
      "malformed",
    ]);
  });
});

describe("checkEd25519Support", () => {
  it("finds Ed25519 where it exists", async () => {
    expect(await checkEd25519Support()).toEqual({ supported: true });
  });

  it("explains itself where it does not", async () => {
    const refuses = {
      importKey: () => Promise.reject(new DOMException("Unrecognized name.", "NotSupportedError")),
    } as unknown as SubtleCrypto;
    const support = await checkEd25519Support(refuses);
    expect(support.supported).toBe(false);
    expect(support.supported === false && support.reason).toContain("Chrome 137");
  });

  it("blames the likeliest cause when there is no WebCrypto at all", async () => {
    const support = await checkEd25519Support(null);
    expect(support.supported === false && support.reason).toContain("HTTPS");
  });
});

describe("fetchJwks", () => {
  it("returns the published keys", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve({
          ok: true,
          status: 200,
          json: () => Promise.resolve({ keys: [{ kid: "live", kty: "OKP" }] }),
        }),
      ),
    );
    expect(await fetchJwks("https://api.example/.well-known/jwks.json")).toEqual([
      { kid: "live", kty: "OKP" },
    ]);
  });

  it("names the URL when the key set cannot be read", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve({ ok: false, status: 503 })));
    await expect(fetchJwks("https://api.example/jwks")).rejects.toThrow("503");

    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve({}) })),
    );
    await expect(fetchJwks("https://api.example/jwks")).rejects.toThrow("keys");
  });
});
