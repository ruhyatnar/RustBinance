//! Ed25519 PKCS#8 PEM authentication and Binance WS API canonical signing (§4.1).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bot_types::Secret;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Map, Value};

use crate::error::GatewayError;

/// Ed25519 API credentials stored exclusively inside [`Secret`] wrappers.
///
/// `Debug` output is always redacted and raw private-key bytes are zeroized on drop.
pub struct Ed25519Credentials {
    api_key: Secret<String>,
    /// Raw 32-byte Ed25519 secret seed, zeroized on drop by `Secret<[u8; 32]>`.
    secret_seed: Secret<[u8; 32]>,
}

impl fmt::Debug for Ed25519Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ed25519Credentials")
            .field("api_key", &self.api_key)
            .field("secret_seed", &self.secret_seed)
            .finish()
    }
}

impl Ed25519Credentials {
    /// Construct credentials from an API key and a PKCS#8 PEM-encoded Ed25519 private key.
    ///
    /// # Errors
    /// Returns [`GatewayError::KeyLoad`] if the API key is empty or the PEM cannot be decoded.
    pub fn from_pkcs8_pem(api_key: String, pem: &str) -> Result<Self, GatewayError> {
        if api_key.trim().is_empty() {
            return Err(GatewayError::KeyLoad("API key must not be empty".into()));
        }
        let signing_key = SigningKey::from_pkcs8_pem(pem)
            .map_err(|_| GatewayError::KeyLoad("invalid Ed25519 PKCS#8 PEM".into()))?;
        let seed = signing_key.to_bytes();
        Ok(Self {
            api_key: Secret::new(api_key),
            secret_seed: Secret::new(seed),
        })
    }

    /// Construct credentials by reading a PKCS#8 PEM file from disk.
    ///
    /// # Errors
    /// Returns [`GatewayError::KeyLoad`] if the file cannot be read or decoded.
    pub fn from_pkcs8_pem_file(api_key: String, path: &Path) -> Result<Self, GatewayError> {
        let pem = std::fs::read_to_string(path).map_err(|err| {
            GatewayError::KeyLoad(format!("unable to read PEM file: {}", err.kind()))
        })?;
        let pem = Secret::new(pem);
        Self::from_pkcs8_pem(api_key, pem.expose())
    }

    /// Construct credentials directly from a 32-byte Ed25519 seed (useful for tests).
    ///
    /// # Errors
    /// Returns [`GatewayError::KeyLoad`] if the API key is empty.
    pub fn from_seed(api_key: String, seed: [u8; 32]) -> Result<Self, GatewayError> {
        if api_key.trim().is_empty() {
            return Err(GatewayError::KeyLoad("API key must not be empty".into()));
        }
        Ok(Self {
            api_key: Secret::new(api_key),
            secret_seed: Secret::new(seed),
        })
    }

    /// Sign a canonical Binance WS API parameter map (alphabetically sorted `k=v&...`)
    /// using Ed25519 and return the Base64-encoded signature (case-sensitive).
    #[must_use]
    pub fn sign_params(&self, params: &BTreeMap<String, String>) -> String {
        let payload = canonical_payload(params);
        let signing_key = SigningKey::from_bytes(self.secret_seed.expose());
        let sig = signing_key.sign(payload.as_bytes());
        BASE64_STANDARD.encode(sig.to_bytes())
    }

    /// Build the signed `params` JSON object for Binance Spot WS API `session.logon`.
    ///
    /// Per official Binance Spot WS API docs, `session.logon` requires `apiKey`, `timestamp`,
    /// optional `recvWindow`, and Base64 Ed25519 `signature` over the alphabetically sorted
    /// query string of all parameters except `signature`.
    #[must_use]
    pub fn build_session_logon_params(
        &self,
        timestamp_ms: i64,
        recv_window_ms: Option<u64>,
    ) -> Value {
        let mut to_sign = BTreeMap::new();
        to_sign.insert("apiKey".to_owned(), self.api_key.expose().clone());
        to_sign.insert("timestamp".to_owned(), timestamp_ms.to_string());
        if let Some(rw) = recv_window_ms {
            to_sign.insert("recvWindow".to_owned(), rw.to_string());
        }
        let signature = self.sign_params(&to_sign);

        let mut obj = Map::new();
        obj.insert(
            "apiKey".to_owned(),
            Value::String(self.api_key.expose().clone()),
        );
        if let Some(rw) = recv_window_ms {
            obj.insert("recvWindow".to_owned(), Value::Number(rw.into()));
        }
        obj.insert("timestamp".to_owned(), Value::Number(timestamp_ms.into()));
        obj.insert("signature".to_owned(), Value::String(signature));
        Value::Object(obj)
    }

    /// Expose the Ed25519 public verifying key bytes (safe to inspect/verify in tests).
    #[must_use]
    pub fn public_key_bytes(&self) -> [u8; 32] {
        let signing_key = SigningKey::from_bytes(self.secret_seed.expose());
        signing_key.verifying_key().to_bytes()
    }
}

/// Build the canonical Binance WS API signing payload: `k1=v1&k2=v2` sorted by key.
#[must_use]
pub fn canonical_payload(params: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (idx, (k, v)) in params.iter().enumerate() {
        if idx > 0 {
            out.push('&');
        }
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
    use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};

    use super::{Ed25519Credentials, canonical_payload};

    #[test]
    fn canonical_payload_sorts_alphabetically() {
        let mut map = BTreeMap::new();
        map.insert("timestamp".into(), "1700000000000".into());
        map.insert("apiKey".into(), "myKey".into());
        map.insert("recvWindow".into(), "5000".into());
        assert_eq!(
            canonical_payload(&map),
            "apiKey=myKey&recvWindow=5000&timestamp=1700000000000"
        );
    }

    #[test]
    fn loads_pkcs8_pem_and_produces_verifiable_logon_signature() {
        let seed = [7u8; 32];
        let sk = SigningKey::from_bytes(&seed);
        let pem = sk.to_pkcs8_pem(LineEnding::LF).expect("encode pkcs8 pem");

        let creds =
            Ed25519Credentials::from_pkcs8_pem("TEST_API_KEY_123".into(), &pem).expect("load pem");
        let params = creds.build_session_logon_params(1_770_736_694_138, Some(5_000));

        assert_eq!(params["apiKey"], "TEST_API_KEY_123");
        assert_eq!(params["recvWindow"], 5_000);
        assert_eq!(params["timestamp"], 1_770_736_694_138i64);

        let sig_b64 = params["signature"].as_str().expect("signature str");
        let sig_bytes = BASE64_STANDARD.decode(sig_b64).expect("base64 decode");
        let sig = Signature::from_slice(&sig_bytes).expect("64-byte ed25519 sig");

        let vk = VerifyingKey::from_bytes(&creds.public_key_bytes()).expect("verifying key");
        let expected_payload = "apiKey=TEST_API_KEY_123&recvWindow=5000&timestamp=1770736694138";
        vk.verify(expected_payload.as_bytes(), &sig)
            .expect("signature must verify against canonical payload");
    }

    #[test]
    fn debug_and_errors_never_leak_secrets() {
        let creds = Ed25519Credentials::from_seed("SUPER_SECRET_API_KEY".into(), [42u8; 32])
            .expect("from seed");
        let dbg = format!("{creds:?}");
        assert!(!dbg.contains("SUPER_SECRET_API_KEY"));
        assert!(dbg.contains("Secret(***)"));

        let err = Ed25519Credentials::from_pkcs8_pem(
            "KEY".into(),
            "-----BEGIN PRIVATE KEY-----\nSECRET_BOGUS_BYTES\n-----END PRIVATE KEY-----",
        )
        .expect_err("invalid pem must fail");
        let err_msg = err.to_string();
        assert!(!err_msg.contains("SECRET_BOGUS_BYTES"));
    }
}
