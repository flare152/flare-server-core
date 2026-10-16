//! Read-only in-memory runtime receipt. Never reads env/files/SQL during a probe.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

#[derive(Clone, Debug, Serialize)]
pub struct LoadedKeys {
    pub keys: BTreeMap<String, String>,
    pub fallback: bool,
}
impl LoadedKeys {
    pub fn context(keys: &crate::auth::KeySet) -> Self {
        Self {
            keys: keys.fingerprints().clone(),
            fallback: keys.is_single_key(),
        }
    }
}
pub type Inventory = BTreeMap<String, LoadedKeys>;
pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Serialize)]
pub struct Receipt {
    pub schema: &'static str,
    pub component: &'static str,
    pub boot_id: String,
    pub nonce: String,
    pub keysets: Inventory,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Unauthorized,
    InvalidNonce,
    Unavailable,
}

pub struct RuntimeProbe {
    component: &'static str,
    boot_id: String,
    token_hash: [u8; 32],
    inventory: Arc<dyn Fn() -> Option<Inventory> + Send + Sync>,
}
impl RuntimeProbe {
    pub fn new(
        component: &'static str,
        token: &str,
        inventory: impl Fn() -> Option<Inventory> + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            matches!(component, "platform" | "social-gateway" | "core-admin"),
            "invalid ops component"
        );
        anyhow::ensure!(
            hex(token, 64),
            "ops read token must be independent 32-byte lowercase hex"
        );
        Ok(Self {
            component,
            boot_id: uuid::Uuid::new_v4().to_string(),
            token_hash: Sha256::digest(token.as_bytes()).into(),
            inventory: Arc::new(inventory),
        })
    }
    /// Absent disables the route; an invalid present value fails startup. No fallback.
    pub fn from_env(
        component: &'static str,
        inventory: impl Fn() -> Option<Inventory> + Send + Sync + 'static,
    ) -> anyhow::Result<Option<Self>> {
        match std::env::var("FLARE_OPS_READ_TOKEN") {
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => anyhow::bail!("invalid ops read token encoding"),
            Ok(mut token) => {
                let result = Self::new(component, &token, inventory);
                token.zeroize();
                result.map(Some)
            }
        }
    }
    pub fn receipt(&self, token: &str, nonce: &str) -> Result<Receipt, Refusal> {
        let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if token.len() != 64 || !bool::from(self.token_hash.ct_eq(&supplied)) {
            return Err(Refusal::Unauthorized);
        }
        if !hex(nonce, 32) {
            return Err(Refusal::InvalidNonce);
        }
        let keysets = (self.inventory)().ok_or(Refusal::Unavailable)?;
        if keysets.is_empty()
            || keysets.len() > 8
            || keysets.iter().any(|(purpose, v)| {
                purpose.is_empty()
                    || purpose.len() > 32
                    || !purpose.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')
                    || v.keys.is_empty()
                    || v.keys.len() > 64
                    || v.keys
                        .iter()
                        .any(|(kid, sha)| !crate::auth::valid_kid(kid) || !hex(sha, 64))
            })
        {
            return Err(Refusal::Unavailable);
        }
        Ok(Receipt {
            schema: "flare.runtime/v1",
            component: self.component,
            boot_id: self.boot_id.clone(),
            nonce: nonce.to_owned(),
            keysets,
        })
    }
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inventory() -> Option<Inventory> {
        Some(BTreeMap::from([(
            "context".into(),
            LoadedKeys {
                keys: BTreeMap::from([("gateway".into(), "a".repeat(64))]),
                fallback: false,
            },
        )]))
    }
    #[test]
    fn nonce_auth_and_live_inventory_are_bounded() {
        let probe = RuntimeProbe::new("core-admin", &"b".repeat(64), inventory).unwrap();
        assert!(matches!(
            probe.receipt(&"c".repeat(64), &"0".repeat(32)),
            Err(Refusal::Unauthorized)
        ));
        assert!(matches!(
            probe.receipt(&"b".repeat(64), "bad"),
            Err(Refusal::InvalidNonce)
        ));
        let a = probe.receipt(&"b".repeat(64), &"0".repeat(32)).unwrap();
        let b = probe.receipt(&"b".repeat(64), &"1".repeat(32)).unwrap();
        assert_eq!(a.boot_id, b.boot_id);
        assert_ne!(a.nonce, b.nonce);
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains(&"b".repeat(64)));
        assert!(RuntimeProbe::new("unexpected", &"b".repeat(64), inventory).is_err());
        let missing = RuntimeProbe::new("platform", &"b".repeat(64), || None).unwrap();
        assert!(matches!(
            missing.receipt(&"b".repeat(64), &"0".repeat(32)),
            Err(Refusal::Unavailable)
        ));
    }
    #[test]
    fn context_fingerprint_is_canonical_and_failed_insert_is_atomic() {
        use base64::Engine;
        use ed25519_dalek::{SigningKey, pkcs8::EncodePublicKey};
        let public = SigningKey::from_bytes(&[41; 32]).verifying_key();
        let pem = public.to_public_key_pem(Default::default()).unwrap();
        let mut set = crate::auth::KeySet::empty();
        set.insert("gateway", &pem).unwrap();
        let original = set.fingerprints().clone();
        assert!(set.insert("gateway", "invalid").is_err());
        assert_eq!(&original, set.fingerprints());
        let encoded = base64::engine::general_purpose::STANDARD.encode(pem.as_bytes());
        set.insert("gateway", &encoded).unwrap();
        assert_eq!(&original, set.fingerprints());
        assert_eq!(
            original["gateway"],
            format!("{:x}", Sha256::digest(public.as_bytes()))
        );
    }
}
