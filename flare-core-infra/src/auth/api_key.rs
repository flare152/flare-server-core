//! Business-neutral high-entropy API Key material. No tenant lookup or IAM policy.
//!
//! Control owns lifecycle/scopes and projects only the digest and public metadata.
//! Each verifier needs the same dedicated 32-byte pepper. Never reuse JWT/Cell keys.
//! Consumers must additionally check binding, revocation, expiry, scopes and source IP.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
pub const API_KEY_ID_LEN: usize = 12;
pub const API_KEY_SECRET_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyKind {
    Live,
    Test,
}

impl ApiKeyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ApiKeyMaterialError {
    #[error("invalid API key format")]
    InvalidToken,
    #[error("invalid API key pepper: expected canonical base64url of 32 random bytes")]
    InvalidPepper,
    #[error("API key entropy source unavailable")]
    EntropyUnavailable,
}

/// Installation-scoped verifier pepper; no defaults, serialization or secret Debug.
pub struct ApiKeyPepper(Zeroizing<[u8; API_KEY_SECRET_LEN]>);

impl ApiKeyPepper {
    pub fn from_base64url(encoded: &str) -> Result<Self, ApiKeyMaterialError> {
        decode_secret(encoded)
            .map(Self)
            .map_err(|_| ApiKeyMaterialError::InvalidPepper)
    }

    /// Issuer/verifier deployments compare this public configuration fingerprint.
    /// A mismatch must fail closed rather than rejecting every key ambiguously.
    pub fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_slice()).into()
    }
}

impl std::fmt::Debug for ApiKeyPepper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKeyPepper([REDACTED])")
    }
}

/// Secret-bearing material is deliberately not Clone/Serialize/Display.
pub struct ApiKeyToken {
    kind: ApiKeyKind,
    key_id: String,
    secret: Zeroizing<[u8; API_KEY_SECRET_LEN]>,
}

impl std::fmt::Debug for ApiKeyToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyToken")
            .field("kind", &self.kind)
            .field("key_id", &self.key_id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

fn decode_secret(encoded: &str) -> Result<Zeroizing<[u8; 32]>, ApiKeyMaterialError> {
    if encoded.len() != 43 {
        return Err(ApiKeyMaterialError::InvalidToken);
    }
    let mut decoded = Zeroizing::new([0u8; 32]);
    let len = URL_SAFE_NO_PAD
        .decode_slice(encoded, decoded.as_mut_slice())
        .map_err(|_| ApiKeyMaterialError::InvalidToken)?;
    // Require one canonical representation, including the unused trailing bits.
    let canonical = Zeroizing::new(URL_SAFE_NO_PAD.encode(decoded.as_slice()));
    if len != 32 || canonical.as_str() != encoded {
        return Err(ApiKeyMaterialError::InvalidToken);
    }
    Ok(decoded)
}

impl ApiKeyToken {
    pub fn parse(encoded: &str) -> Result<Self, ApiKeyMaterialError> {
        // Fixed size also bounds work on attacker-controlled input.
        if encoded.len() != 65 {
            return Err(ApiKeyMaterialError::InvalidToken);
        }
        // The base64url secret itself may contain underscores.
        let mut fields = encoded.splitn(4, '_');
        if fields.next() != Some("flr") {
            return Err(ApiKeyMaterialError::InvalidToken);
        }
        let kind = match fields.next() {
            Some("live") => ApiKeyKind::Live,
            Some("test") => ApiKeyKind::Test,
            _ => return Err(ApiKeyMaterialError::InvalidToken),
        };
        let key_id = fields.next().ok_or(ApiKeyMaterialError::InvalidToken)?;
        if key_id.len() != API_KEY_ID_LEN || !key_id.bytes().all(|b| BASE32.contains(&b)) {
            return Err(ApiKeyMaterialError::InvalidToken);
        }
        let secret = decode_secret(fields.next().ok_or(ApiKeyMaterialError::InvalidToken)?)?;
        Ok(Self {
            kind,
            key_id: key_id.into(),
            secret,
        })
    }

    pub fn generate(kind: ApiKeyKind) -> Result<Self, ApiKeyMaterialError> {
        let mut id_entropy = [0u8; 8];
        let mut secret = Zeroizing::new([0u8; API_KEY_SECRET_LEN]);
        OsRng
            .try_fill_bytes(&mut id_entropy)
            .and_then(|_| OsRng.try_fill_bytes(secret.as_mut_slice()))
            .map_err(|_| ApiKeyMaterialError::EntropyUnavailable)?;
        let bits = u64::from_be_bytes(id_entropy);
        let key_id = (0..API_KEY_ID_LEN)
            .map(|i| BASE32[((bits >> (59 - i * 5)) & 31) as usize] as char)
            .collect();
        Ok(Self {
            kind,
            key_id,
            secret,
        })
    }

    pub fn kind(&self) -> ApiKeyKind {
        self.kind
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Only issuance responses expose this value. Persistence never stores it.
    /// The caller controls its lifetime; the returned buffer is zeroized on drop.
    pub fn into_issuance_token(self) -> Zeroizing<String> {
        let encoded_secret = Zeroizing::new(URL_SAFE_NO_PAD.encode(self.secret.as_slice()));
        Zeroizing::new(format!(
            "flr_{}_{}_{}",
            self.kind.as_str(),
            self.key_id,
            encoded_secret.as_str()
        ))
    }

    /// Exactly SHA-256(pepper bytes || decoded 32-byte secret).
    pub fn digest(&self, pepper: &ApiKeyPepper) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(pepper.0.as_slice());
        digest.update(self.secret.as_slice());
        digest.finalize().into()
    }

    /// Verify public selector/kind too: changing live/test must never reuse a hash.
    /// Lifecycle/binding/policy checks remain the consuming domain's responsibility.
    pub fn verify(
        &self,
        pepper: &ApiKeyPepper,
        expected_kind: ApiKeyKind,
        expected_key_id: &str,
        expected_digest: &[u8; 32],
    ) -> bool {
        let matches = bool::from(self.digest(pepper).ct_eq(expected_digest));
        matches && self.kind == expected_kind && self.key_id == expected_key_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pepper(byte: u8) -> ApiKeyPepper {
        ApiKeyPepper::from_base64url(&URL_SAFE_NO_PAD.encode([byte; 32])).unwrap()
    }

    fn token(kind: &str, secret: [u8; 32]) -> String {
        format!("flr_{kind}_ABCDEFGH2345_{}", URL_SAFE_NO_PAD.encode(secret))
    }

    #[test]
    fn base64url_underscores_are_secret_not_field_delimiters() {
        let raw = token("live", [255; 32]);
        assert!(raw.matches('_').count() > 3);
        let parsed = ApiKeyToken::parse(&raw).unwrap();
        assert_eq!(parsed.kind(), ApiKeyKind::Live);
        assert_eq!(parsed.key_id(), "ABCDEFGH2345");
        assert_eq!(*parsed.into_issuance_token(), raw);
    }

    #[test]
    fn rejects_noncanonical_malformed_and_oversized_keys() {
        let valid = token("live", [0; 32]);
        for raw in [
            "".into(),
            valid.to_lowercase(),
            valid.replace("flr_", "FLR_"),
            valid.replace("live", "prod"),
            valid.replace("ABCDEFGH2345", "ABCDEFGH2341"),
            valid.replace("ABCDEFGH2345", "ABCDEFGH234"),
            format!("{valid}="),
            format!(" {valid}"),
            format!("{valid}\n"),
            "x".repeat(1_000_000),
            format!("{}B", &valid[..64]), // Same payload, invalid trailing bits.
        ] {
            assert_eq!(
                ApiKeyToken::parse(&raw).unwrap_err(),
                ApiKeyMaterialError::InvalidToken
            );
        }
    }

    #[test]
    fn hashes_raw_secret_bytes_with_a_fixed_known_vector() {
        let key = ApiKeyToken::parse(&token("live", [255; 32])).unwrap();
        let expected = [
            0xd8, 0x44, 0x0c, 0x95, 0xa6, 0x11, 0xdc, 0x85, 0x00, 0xe8, 0xc9, 0x36, 0x5c, 0xab,
            0xc7, 0xfe, 0x07, 0xa5, 0x28, 0x02, 0x3b, 0x19, 0x10, 0x2d, 0x52, 0xbe, 0x3f, 0x75,
            0x0c, 0xee, 0x68, 0x92,
        ];
        assert_eq!(key.digest(&pepper(7)), expected);
    }

    #[test]
    fn verification_binds_kind_id_pepper_and_every_digest_byte() {
        let key = ApiKeyToken::parse(&token("live", [42; 32])).unwrap();
        let p = pepper(7);
        let hash = key.digest(&p);
        assert!(key.verify(&p, ApiKeyKind::Live, key.key_id(), &hash));
        assert!(!key.verify(&p, ApiKeyKind::Test, key.key_id(), &hash));
        assert!(!key.verify(&p, ApiKeyKind::Live, "OTHERKEY2345", &hash));
        assert!(!key.verify(&pepper(8), ApiKeyKind::Live, key.key_id(), &hash));
        for index in 0..32 {
            let mut corrupt = hash;
            corrupt[index] ^= 1;
            assert!(!key.verify(&p, ApiKeyKind::Live, key.key_id(), &corrupt));
        }
    }

    #[test]
    fn pepper_requires_canonical_exact_length_without_fallback() {
        for raw in [
            "".into(),
            "short".into(),
            URL_SAFE_NO_PAD.encode([0; 31]),
            URL_SAFE_NO_PAD.encode([0; 33]),
            format!("{}=", URL_SAFE_NO_PAD.encode([0; 32])),
        ] {
            assert_eq!(
                ApiKeyPepper::from_base64url(&raw).unwrap_err(),
                ApiKeyMaterialError::InvalidPepper
            );
        }
        assert_ne!(pepper(7).fingerprint(), pepper(8).fingerprint());
    }

    #[test]
    fn logs_errors_and_debug_never_disclose_secret_material() {
        let raw = token("live", [42; 32]);
        let secret = raw.splitn(4, '_').nth(3).unwrap();
        let key = ApiKeyToken::parse(&raw).unwrap();
        assert!(!format!("{key:?}").contains(secret));
        assert!(!format!("{:?}", pepper(7)).contains(&URL_SAFE_NO_PAD.encode([7; 32])));
        let error = ApiKeyToken::parse(&format!("{raw}!")).unwrap_err();
        assert!(!error.to_string().contains(secret));
    }

    #[test]
    fn generation_produces_exact_wire_shape_and_distinct_random_secrets() {
        let p = pepper(7);
        let mut ids = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();
        for kind in [ApiKeyKind::Live, ApiKeyKind::Test] {
            for _ in 0..64 {
                let key = ApiKeyToken::generate(kind).unwrap();
                let key_id = key.key_id().to_string();
                let hash = key.digest(&p);
                assert!(ids.insert(key_id.clone()));
                assert!(hashes.insert(hash));
                let raw = key.into_issuance_token();
                assert_eq!(raw.len(), 65);
                let parsed = ApiKeyToken::parse(&raw).unwrap();
                assert!(parsed.verify(&p, kind, &key_id, &hash));
            }
        }
    }
}
