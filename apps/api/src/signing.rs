//! The deployment's token-signing identity (roadmap #52).
//!
//! The resource-server scenario used to sign with a **per-session HMAC
//! secret** and validate with the same secret in the same process. That
//! demonstrates that a 401 can be produced; it does not demonstrate a resource
//! server, because nothing is discovered and the validating side holds the key
//! that minted the token.
//!
//! So the deployment signs **asymmetrically** and publishes the public half at
//! `/.well-known/jwks.json`. The validating side then holds no secret at all —
//! only a URL and a `kid` to look up, which is what a real resource server
//! has. That property is impossible to show with HS256, and it is the whole
//! reason this module exists.
//!
//! ## Ed25519 rather than RSA
//!
//! `TokenManager` offers both (`new_ed25519`, `new_asymmetric`). Ed25519 wins
//! here on two counts: generating a key is 32 bytes of randomness rather than
//! a prime search measured in seconds, which matters because this deployment
//! generates one at boot when it has not been given one; and the published JWK
//! is the compact OKP shape from RFC 8037, which a visitor can read at a glance
//! next to the `kid` in their token's header.
//!
//! ## Per deployment, not per session
//!
//! A JWKS has to be fetchable, so the key cannot be per-visitor the way the
//! old HMAC secret was. One key for the process, published once. The cost is
//! that every visitor's token validates against the same key — which is
//! exactly right: they are all talking to one issuer.

use std::sync::Arc;

use authkestra_engine::token::jwk::Jwk;
use authkestra_engine::TokenManager;

/// Where the public key is published, relative to the API's own base URL.
///
/// The well-known path rather than a bespoke one: a resource server pointed at
/// an issuer expects to find keys where the RFCs say they live, and the
/// scenario is worth nothing if the URL is a playground invention.
pub const JWKS_PATH: &str = "/.well-known/jwks.json";

/// The signing identity of this deployment.
pub struct SigningKeys {
    manager: Arc<TokenManager>,
    /// The same key as `manager` holds, kept separately so an arbitrary claim
    /// set can be signed. See [`SigningKeys::sign`].
    encoding: jsonwebtoken::EncodingKey,
    jwk: Jwk,
    issuer: String,
    jwks_url: String,
}

impl SigningKeys {
    /// Read the key from the environment, or generate one.
    ///
    /// `TOKEN_SIGNING_KEY_PEM` is an Ed25519 private key in PKCS#8 PEM, as
    /// produced by `openssl genpkey -algorithm ed25519`. Literal `\n`
    /// sequences are unescaped first, because a PEM is multi-line and most
    /// hosting dashboards only take a single line.
    ///
    /// Generating a key when none is supplied keeps `cargo run` working with
    /// no setup, at the cost of every restart invalidating outstanding tokens.
    /// For a demo whose sessions last twelve hours that is a fair trade, but it
    /// is logged loudly, because "my token stopped working" is otherwise a
    /// mystery.
    pub fn from_env(issuer: String) -> Self {
        let supplied = std::env::var("TOKEN_SIGNING_KEY_PEM")
            .ok()
            .map(|raw| raw.replace("\\n", "\n"))
            .filter(|pem| !pem.trim().is_empty());

        let pem = match supplied {
            Some(pem) => {
                tracing::info!("token signing key loaded from TOKEN_SIGNING_KEY_PEM");
                pem
            }
            None => {
                tracing::warn!(
                    "TOKEN_SIGNING_KEY_PEM is not set; generating a signing key for this \
                     process. Tokens already issued stop validating on restart, and two \
                     instances will publish different keys — set it in any deployment \
                     that runs more than one."
                );
                generate_ed25519_pem()
            }
        };

        // A supplied key that cannot be parsed must not fall back to a
        // generated one: the deployment asked for a specific signing identity,
        // and quietly substituting another would mean tokens that validate
        // here and nowhere else, with no symptom until something downstream
        // rejects them.
        let manager = TokenManager::new_ed25519(pem.as_bytes(), Some(issuer.clone()), None)
            .unwrap_or_else(|e| {
                panic!(
                    "TOKEN_SIGNING_KEY_PEM is not a usable Ed25519 PKCS#8 PEM: {e}. \
                     Generate one with `openssl genpkey -algorithm ed25519`."
                )
            });

        // `new_ed25519` always populates this; the Option is for the HS256
        // constructor, which has no public half to publish.
        let jwk = manager
            .public_jwk()
            .expect("an Ed25519 TokenManager always carries its public JWK");

        tracing::info!(
            kid = ?jwk.kid,
            alg = ?jwk.alg,
            issuer = %issuer,
            "token signing identity"
        );

        let jwks_url = format!("{}{JWKS_PATH}", issuer.trim_end_matches('/'));
        Self {
            manager: Arc::new(manager),
            encoding: encoding_key(&pem),
            jwk,
            issuer,
            jwks_url,
        }
    }

    /// A key for tests, generated in-process.
    pub fn for_test(issuer: &str) -> Self {
        let pem = generate_ed25519_pem();
        let manager = TokenManager::new_ed25519(pem.as_bytes(), Some(issuer.to_string()), None)
            .expect("a freshly generated key parses");
        let jwk = manager.public_jwk().expect("Ed25519 carries a JWK");
        Self {
            manager: Arc::new(manager),
            encoding: encoding_key(&pem),
            jwk,
            jwks_url: format!("{}{JWKS_PATH}", issuer.trim_end_matches('/')),
            issuer: issuer.to_string(),
        }
    }

    /// Signs this deployment's tokens.
    pub fn manager(&self) -> &Arc<TokenManager> {
        &self.manager
    }

    /// The `iss` every token carries, and the name a validator must trust.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Where a resource server fetches the public half.
    pub fn jwks_url(&self) -> &str {
        &self.jwks_url
    }

    /// The JWKS document, as served.
    ///
    /// A `keys` array with one entry rather than the bare JWK: that is the
    /// shape RFC 7517 defines and the shape `authkestra-resource` parses, and a
    /// single-key issuer is still a set.
    pub fn jwks(&self) -> serde_json::Value {
        serde_json::json!({ "keys": [self.jwk.clone()] })
    }

    /// The `kid` a visitor should find in both their token and the JWKS.
    pub fn kid(&self) -> Option<&str> {
        self.jwk.kid.as_deref()
    }

    /// Sign an arbitrary claim set with this deployment's key and `kid`.
    ///
    /// An issuer's fundamental operation, and exposed because
    /// `TokenManager`'s `issue_*` methods take an **unsigned** lifetime — so
    /// "expired one second ago" is not expressible through them, and the
    /// resource scenario needs exactly that: a token this deployment really
    /// signed, correctly, that is nonetheless past its `exp`. Anything else
    /// would demonstrate expiry by waiting for it.
    pub fn sign(&self, claims: &serde_json::Value) -> Result<String, jsonwebtoken::errors::Error> {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
        header.kid = self.jwk.kid.clone();
        jsonwebtoken::encode(&header, claims, &self.encoding)
    }
}

/// The signing half of a PEM this module has already parsed once.
///
/// Panics on failure by design: both call sites have just built a
/// `TokenManager` from the same bytes, so a failure here would mean the two
/// halves disagree about a key that is already in use.
fn encoding_key(pem: &str) -> jsonwebtoken::EncodingKey {
    jsonwebtoken::EncodingKey::from_ed_pem(pem.as_bytes())
        .expect("a PEM that built a TokenManager also builds an EncodingKey")
}

/// A fresh Ed25519 private key, PKCS#8 PEM encoded.
///
/// Public because the resource scenario needs throwaway keys of its own, to
/// mint tokens that are *meant* to fail validation. Those are not signing
/// identities and deliberately do not go through [`SigningKeys`].
pub fn generate_ed25519_pem() -> String {
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    use rand_core::RngCore;

    // `SigningKey::generate` needs dalek's `rand_core` feature, which pins a
    // particular `rand_core` major. Filling the bytes ourselves and using
    // `from_bytes` needs no feature and no version agreement — the key is 32
    // bytes of randomness either way.
    let mut seed = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);

    signing
        .to_pkcs8_pem(ed25519_dalek::pkcs8::spki::der::pem::LineEnding::LF)
        .expect("a dalek signing key always encodes to PKCS#8")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_key_round_trips_into_a_token_manager() {
        let keys = SigningKeys::for_test("https://issuer.test");
        assert_eq!(keys.issuer(), "https://issuer.test");
        assert_eq!(keys.jwks_url(), "https://issuer.test/.well-known/jwks.json");
        assert!(keys.kid().is_some(), "every token needs a kid to look up");
    }

    /// The published document has to be the shape a resource server parses,
    /// and it must never carry the private half.
    #[test]
    fn the_jwks_is_a_public_key_set() {
        let keys = SigningKeys::for_test("https://issuer.test");
        let doc = keys.jwks();

        let entries = doc["keys"].as_array().expect("a `keys` array");
        assert_eq!(entries.len(), 1);
        let key = &entries[0];
        assert_eq!(key["kty"], "OKP");
        assert_eq!(key["crv"], "Ed25519");
        assert_eq!(key["alg"], "EdDSA");
        assert!(key["x"].is_string(), "the public point must be published");
        assert_eq!(key["kid"].as_str(), keys.kid());

        // Nothing that could sign. `d` is the Ed25519 private scalar; the RSA
        // private fields are checked too, since the Jwk type carries both
        // shapes and only omits what is `None`.
        let serialised = doc.to_string();
        for private in ["\"d\"", "\"p\"", "\"q\"", "\"dp\"", "\"dq\"", "\"qi\""] {
            assert!(
                !serialised.contains(private),
                "the JWKS leaked a private field {private}: {serialised}"
            );
        }
        assert!(!serialised.contains("PRIVATE KEY"));
    }

    /// The token has to name the key that signed it, or a validator has
    /// nothing to look up and `require_kid` refuses it outright.
    #[test]
    fn issued_tokens_carry_the_published_kid_and_issuer() {
        let keys = SigningKeys::for_test("https://issuer.test");
        let token = keys
            .manager()
            .issue_user_token(
                authkestra_engine::Identity {
                    provider_id: "playground".to_string(),
                    external_id: "visitor-1".to_string(),
                    email: None,
                    username: None,
                    attributes: std::collections::HashMap::new(),
                },
                300,
                None,
                Some("playground-api".to_string()),
            )
            .expect("token issues");

        let header = jsonwebtoken::decode_header(&token).expect("a readable header");
        assert_eq!(header.alg, jsonwebtoken::Algorithm::EdDSA);
        assert_eq!(
            header.kid.as_deref(),
            keys.kid(),
            "the header's kid must be the one published in the JWKS"
        );
    }

    /// Two deployments must not accidentally share a signing identity.
    #[test]
    fn each_generated_key_is_distinct() {
        let a = SigningKeys::for_test("https://a.test");
        let b = SigningKeys::for_test("https://b.test");
        assert_ne!(a.kid(), b.kid());
        assert_ne!(a.jwks()["keys"][0]["x"], b.jwks()["keys"][0]["x"]);
    }

    /// A trailing slash on the issuer must not produce a double slash in the
    /// URL a validator is handed.
    #[test]
    fn the_jwks_url_survives_a_trailing_slash() {
        let keys = SigningKeys::for_test("https://issuer.test/");
        assert_eq!(keys.jwks_url(), "https://issuer.test/.well-known/jwks.json");
    }
}
