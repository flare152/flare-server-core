//! Validated, bounded local credential projections. No transport/DB/tenant state.
//! Serialization below is deliberately limited to hash-only storage records;
//! the validated snapshot and grant never expose token, secret or pepper.
use super::{
    api_key::ApiKeyKind,
    api_key_policy::{ApiKeyGrant, ApiKeyPolicyError},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_CREDENTIAL_KEYS: usize = 1024;
pub const MAX_CREDENTIAL_SNAPSHOT_BYTES: usize = 1024 * 1024;

/// Untrusted storage/transport input. Always validate through CredentialSnapshot
/// (or grant with its trusted application binding) before using it for auth.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialKeyRecord {
    pub key_id: String,
    pub service_account_id: String,
    /// 1=live, 2=test. Zero/unknown MUST NOT default to live.
    pub mode: u8,
    pub hash: [u8; 32],
    pub pepper_fingerprint: [u8; 32],
    pub scopes: Vec<String>,
    pub expires_at_ms: i64,
    pub ip_allow: Vec<String>,
    pub revoked: bool,
    pub service_account_enabled: bool,
}

impl std::fmt::Debug for CredentialKeyRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialKeyRecord")
            .field("key_id", &self.key_id)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

impl CredentialKeyRecord {
    pub fn grant(&self, app_id: &str) -> Result<ApiKeyGrant, ApiKeyPolicyError> {
        let kind = match self.mode {
            1 => ApiKeyKind::Live,
            2 => ApiKeyKind::Test,
            _ => return Err(ApiKeyPolicyError::InvalidProjection),
        };
        if self.expires_at_ms < 0 {
            return Err(ApiKeyPolicyError::InvalidProjection);
        }
        ApiKeyGrant::new(
            self.key_id.clone(),
            app_id.into(),
            self.service_account_id.clone(),
            kind,
            self.hash,
            self.pepper_fingerprint,
            self.scopes.clone(),
            (self.expires_at_ms > 0).then_some(self.expires_at_ms),
            self.revoked,
            self.service_account_enabled,
            self.ip_allow.clone(),
        )
    }
}

#[derive(Clone)]
pub struct CredentialSnapshot {
    app_id: String,
    version: i64,
    keys: Vec<CredentialKeyRecord>,
}

impl std::fmt::Debug for CredentialSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialSnapshot")
            .field("app_id", &self.app_id)
            .field("version", &self.version)
            .field("key_count", &self.keys.len())
            .field("material", &"[REDACTED]")
            .finish()
    }
}

impl CredentialSnapshot {
    pub fn new(
        app_id: String,
        version: u64,
        keys: Vec<CredentialKeyRecord>,
    ) -> Result<Self, ApiKeyPolicyError> {
        if app_id.is_empty()
            || app_id.len() > 32
            || !app_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || version == 0
            || version > i64::MAX as u64
            || keys.len() > MAX_CREDENTIAL_KEYS
        {
            return Err(ApiKeyPolicyError::InvalidProjection);
        }
        let mut ids = BTreeSet::new();
        for key in &keys {
            key.grant(&app_id)?;
            if !ids.insert(&key.key_id) {
                return Err(ApiKeyPolicyError::InvalidProjection);
            }
        }
        // Bound persisted JSON too: up to 1024 individually valid keys can
        // otherwise carry ~3 MiB of scopes/CIDRs in a single transaction.
        let bytes = serde_json::to_vec(&keys).map_err(|_| ApiKeyPolicyError::InvalidProjection)?;
        if bytes.len() > MAX_CREDENTIAL_SNAPSHOT_BYTES {
            return Err(ApiKeyPolicyError::InvalidProjection);
        }
        Ok(Self {
            app_id,
            version: version as i64,
            keys,
        })
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }
    pub fn version(&self) -> i64 {
        self.version
    }
    pub fn keys(&self) -> &[CredentialKeyRecord] {
        &self.keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::api_key::{ApiKeyPepper, ApiKeyToken};
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    fn key() -> CredentialKeyRecord {
        CredentialKeyRecord {
            key_id: "ABCDEFGHIJKL".into(),
            service_account_id: "sa-a".into(),
            mode: 1,
            hash: [42; 32],
            pepper_fingerprint: [43; 32],
            scopes: vec!["endpoint.read".into()],
            expires_at_ms: 0,
            ip_allow: vec![],
            revoked: false,
            service_account_enabled: true,
        }
    }
    fn valid(key: CredentialKeyRecord) -> bool {
        CredentialSnapshot::new("app-a".into(), 1, vec![key]).is_ok()
    }

    #[test]
    fn empty_snapshot_keeps_version_and_binding_without_wildcard_or_overflow() {
        let p = CredentialSnapshot::new("app-a".into(), 9, vec![]).unwrap();
        assert_eq!(p.version(), 9);
        assert_eq!(p.app_id(), "app-a");
        assert!(p.keys().is_empty());
        for app in ["", "*", "default ", " app-a", "app.a"] {
            assert!(CredentialSnapshot::new(app.into(), 9, vec![]).is_err());
        }
        for version in [0, i64::MAX as u64 + 1, u64::MAX] {
            assert!(CredentialSnapshot::new("app-a".into(), version, vec![]).is_err());
        }
    }

    #[test]
    fn validation_is_all_or_nothing_and_duplicate_ids_are_not_last_write_wins() {
        assert!(valid(key()));
        for mode in [0, 3, 255] {
            let mut k = key();
            k.mode = mode;
            assert!(!valid(k));
        }
        let mut k = key();
        k.expires_at_ms = -1;
        assert!(!valid(k));
        let mut k = key();
        k.scopes = vec!["*".into()];
        assert!(!valid(k));
        let mut k = key();
        k.ip_allow = vec!["192.0.2.1/24".into()];
        assert!(!valid(k));
        let mut k = key();
        k.key_id = "bad".into();
        assert!(!valid(k));
        let mut k = key();
        k.service_account_id = "*".into();
        assert!(!valid(k));
        assert!(CredentialSnapshot::new("app-a".into(), 1, vec![key(), key()]).is_err());
        assert!(CredentialSnapshot::new("app-a".into(), 1, vec![key(); 1025]).is_err());
    }

    #[test]
    fn payload_budget_and_redacted_debug_are_independent_of_key_count() {
        let mut keys = Vec::new();
        for i in 0..1024 {
            let mut k = key();
            k.key_id = format!(
                "ABCDEFGHIJ{}{}",
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"[i / 32] as char,
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"[i % 32] as char
            );
            k.scopes = (0..16)
                .map(|n| format!("{}{}", "r".repeat(124), n))
                .collect();
            keys.push(k);
        }
        assert!(CredentialSnapshot::new("app-a".into(), 1, keys).is_err());
        let p = CredentialSnapshot::new("app-a".into(), 1, vec![key()]).unwrap();
        let debug = format!("{p:?} {:?}", p.keys()[0]);
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("42, 42") && !debug.contains("43, 43"));
    }

    #[test]
    fn hash_only_storage_roundtrip_does_not_bypass_disabled_or_revoked_policy() {
        let pepper = ApiKeyPepper::from_base64url(&URL_SAFE_NO_PAD.encode([7; 32])).unwrap();
        let token = ApiKeyToken::generate(ApiKeyKind::Test).unwrap();
        let mut k = key();
        k.key_id = token.key_id().into();
        k.mode = 2;
        k.hash = token.digest(&pepper);
        k.pepper_fingerprint = pepper.fingerprint();
        let bytes = serde_json::to_vec(&k).unwrap();
        assert!(!std::str::from_utf8(&bytes).unwrap().contains("flr_test_"));
        let mut restored: CredentialKeyRecord = serde_json::from_slice(&bytes).unwrap();
        let authorize = |k: &CredentialKeyRecord| {
            k.grant("app-a").unwrap().authorize(
                &token,
                &pepper,
                Some("app-a"),
                "endpoint.read",
                "127.0.0.1".parse().unwrap(),
                1000,
            )
        };
        assert!(authorize(&restored).is_ok());
        restored.service_account_enabled = false;
        assert_eq!(
            authorize(&restored),
            Err(ApiKeyPolicyError::Unauthenticated)
        );
        restored.service_account_enabled = true;
        restored.revoked = true;
        assert_eq!(
            authorize(&restored),
            Err(ApiKeyPolicyError::Unauthenticated)
        );
        assert!(serde_json::from_str::<CredentialKeyRecord>("{}").is_err());
    }
}
