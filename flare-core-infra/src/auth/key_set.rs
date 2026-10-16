//! Cell public-key set for the signed internal context (`x-flare-ctx`), keyed by the JWT `kid`
//! header (v4 R3-1). Every component that signs the context declares the key it signs with;
//! receivers hold the public half of every component key and may pin an actor type to key ids
//! (Core admin surfaces accept `Staff` only from `kid=gateway`).
//!
//! A deployment that still runs one Cell key pair loads it as [`LEGACY_KID`]. Until the
//! per-component keys exist (flarectl, R6-2) a token naming any other kid is verified with that
//! key too, so the pinning is nominal there; R6-4 removes the fallback.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use jsonwebtoken::DecodingKey;

use super::token::ed25519_public_decoding_key;

/// The single Cell public key (SPKI PEM, base64 PEM or base64 DER), registered as [`LEGACY_KID`].
pub const INTERNAL_CTX_PUBLIC_KEY_ENV: &str = "FLARE_INTERNAL_CTX_PUBLIC_KEY";
/// The component key set: a JSON object `{"<kid>": "<key material>"}`, or the path of a directory
/// holding one `<kid>.pem` (or `<kid>.pub`) file per component.
pub const INTERNAL_CTX_PUBLIC_KEYS_ENV: &str = "FLARE_INTERNAL_CTX_PUBLIC_KEYS";
/// Key id of the single-key deployment, and of tokens whose header names no kid.
pub const LEGACY_KID: &str = "legacy";

/// Key ids of the components that sign the internal context.
pub mod kid {
    /// The Social gateway: user requests and the admin partition's Staff context.
    pub const GATEWAY: &str = "gateway";
    /// Hook executors calling back into the Cell.
    pub const HOOK: &str = "hook";
    /// The Social IM bridge.
    pub const BRIDGE: &str = "bridge";
    /// Core message ingest.
    pub const INGEST: &str = "ingest";
    /// Core admin-gateway (re-signs the verified Staff for Media).
    pub const ADMIN_GATEWAY: &str = "admin-gateway";
    /// The Core Platform API machine sender; never resolved by a legacy Cell key.
    pub const API_GATEWAY: &str = "api-gateway";
}

/// Key ids are short ASCII names: letters, digits, `_` and `-`, at most 32 bytes.
pub fn valid_kid(kid: &str) -> bool {
    !kid.is_empty()
        && kid.len() <= 32
        && kid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Public keys by key id. Construction never grants signing capability.
#[derive(Clone)]
pub struct KeySet {
    keys: BTreeMap<String, DecodingKey>,
    fingerprints: BTreeMap<String, String>,
    single: bool,
}

impl std::fmt::Debug for KeySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeySet")
            .field("kids", &self.kids())
            .field("single", &self.single)
            .finish()
    }
}

impl KeySet {
    /// The single-key deployment: `material` is registered as [`LEGACY_KID`] and verifies every
    /// kid until another key is inserted.
    pub fn single(material: &str) -> Result<Self> {
        let mut keys = BTreeMap::new();
        keys.insert(
            LEGACY_KID.to_owned(),
            ed25519_public_decoding_key(material)?,
        );
        let fingerprints = BTreeMap::from([(
            LEGACY_KID.to_owned(),
            super::token::public_key_fingerprint(material)?,
        )]);
        Ok(Self {
            keys,
            fingerprints,
            single: true,
        })
    }

    pub fn empty() -> Self {
        Self {
            keys: BTreeMap::new(),
            fingerprints: BTreeMap::new(),
            single: false,
        }
    }

    /// Registers (or replaces) the key for `kid`. The set stops being a single-key set.
    pub fn insert(&mut self, kid: &str, material: &str) -> Result<()> {
        if !valid_kid(kid) {
            bail!("invalid signed context key id");
        }
        let decoding = ed25519_public_decoding_key(material)?;
        let fingerprint = super::token::public_key_fingerprint(material)?;
        self.keys.insert(kid.to_owned(), decoding);
        self.fingerprints.insert(kid.to_owned(), fingerprint);
        self.single = false;
        Ok(())
    }

    /// `{"<kid>": "<key material>"}`.
    pub fn from_json(json: &str) -> Result<Self> {
        let entries: BTreeMap<String, String> = serde_json::from_str(json).map_err(|_| {
            anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV} must be a JSON object of key id to public key")
        })?;
        if entries.is_empty() {
            bail!("{INTERNAL_CTX_PUBLIC_KEYS_ENV} must name at least one key");
        }
        let mut set = Self::empty();
        for (kid, material) in &entries {
            set.insert(kid, material)
                .map_err(|error| anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV}[{kid}]: {error}"))?;
        }
        Ok(set)
    }

    /// One `<kid>.pem` or `<kid>.pub` file per component; other files are ignored.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let entries = std::fs::read_dir(dir)
            .map_err(|_| anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV} directory is not readable"))?;
        let mut set = Self::empty();
        for entry in entries {
            let path = entry
                .map_err(|_| anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV} directory is not readable"))?
                .path();
            let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !matches!(extension, "pem" | "pub") {
                continue;
            }
            let kid = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("invalid signed context key file name"))?
                .to_owned();
            let material = std::fs::read_to_string(&path).map_err(|_| {
                anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV}[{kid}]: unreadable key file")
            })?;
            set.insert(&kid, material.trim())
                .map_err(|error| anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV}[{kid}]: {error}"))?;
        }
        if set.keys.is_empty() {
            bail!("{INTERNAL_CTX_PUBLIC_KEYS_ENV} directory holds no .pem or .pub key");
        }
        Ok(set)
    }

    /// [`INTERNAL_CTX_PUBLIC_KEYS_ENV`] (JSON or directory) layered over
    /// [`INTERNAL_CTX_PUBLIC_KEY_ENV`] (the legacy key). `None` when neither is set.
    pub fn from_env() -> Result<Option<Self>> {
        Self::from_sources(
            env_text(INTERNAL_CTX_PUBLIC_KEYS_ENV)?.as_deref(),
            env_text(INTERNAL_CTX_PUBLIC_KEY_ENV)?.as_deref(),
        )
    }

    /// [`Self::from_env`] on explicit values: `set` is the key-set spec, `single` the legacy key.
    pub fn from_sources(set: Option<&str>, single: Option<&str>) -> Result<Option<Self>> {
        let set = set.map(str::trim).filter(|v| !v.is_empty());
        let single = single.map(str::trim).filter(|v| !v.is_empty());
        match (set, single) {
            (None, None) => Ok(None),
            (None, Some(single)) => Self::single(single).map(Some),
            (Some(spec), single) => {
                let mut keys = if spec.starts_with('{') {
                    Self::from_json(spec)?
                } else {
                    Self::from_dir(Path::new(spec))?
                };
                if let Some(single) = single
                    && !keys.keys.contains_key(LEGACY_KID)
                {
                    keys.insert(LEGACY_KID, single)?;
                }
                Ok(Some(keys))
            }
        }
    }

    pub fn kids(&self) -> Vec<&str> {
        self.keys.keys().map(String::as_str).collect()
    }

    pub fn fingerprints(&self) -> &BTreeMap<String, String> {
        &self.fingerprints
    }

    pub fn contains(&self, kid: &str) -> bool {
        self.keys.contains_key(kid)
    }

    /// Whether this is the old single-key deployment. New machine capabilities
    /// still require their explicitly named public key.
    pub fn is_single_key(&self) -> bool {
        self.single
    }

    /// The key for a token header: no kid is the legacy key; an unknown kid resolves only in the
    /// single-key deployment, except the independent Platform machine identity.
    pub(crate) fn resolve(&self, kid: Option<&str>) -> Option<&DecodingKey> {
        match kid {
            None => self.keys.get(LEGACY_KID),
            Some(kid::API_GATEWAY) => self.keys.get(kid::API_GATEWAY),
            Some(kid) => self
                .keys
                .get(kid)
                .or_else(|| self.single.then(|| self.keys.get(LEGACY_KID)).flatten()),
        }
    }
}

fn env_text(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(anyhow!("{name} has invalid encoding")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use ed25519_dalek::pkcs8::EncodePublicKey;

    fn public_pem(seed: u8) -> String {
        SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .to_public_key_pem(Default::default())
            .unwrap()
    }

    #[test]
    fn kid_syntax_is_short_ascii() {
        for kid in ["gateway", "admin-gateway", "iam_1", "a"] {
            assert!(valid_kid(kid), "{kid}");
        }
        for kid in ["", " gateway", "gate way", "k/1", "k.1", &"k".repeat(33)] {
            assert!(!valid_kid(kid), "{kid:?}");
        }
    }

    #[test]
    fn single_key_resolves_every_kid_until_a_component_key_is_added() {
        let mut set = KeySet::single(&public_pem(1)).unwrap();
        assert!(set.is_single_key());
        assert_eq!(set.kids(), vec![LEGACY_KID]);
        assert!(set.resolve(None).is_some());
        assert!(set.resolve(Some("gateway")).is_some());
        assert!(
            set.resolve(Some(kid::API_GATEWAY)).is_none(),
            "machine writes cannot borrow the legacy key"
        );
        set.insert("hook", &public_pem(2)).unwrap();
        assert!(!set.is_single_key());
        assert!(set.resolve(Some("hook")).is_some());
        assert!(set.resolve(Some("gateway")).is_none(), "fallback ended");
        assert!(set.resolve(None).is_some(), "legacy stays");
        assert!(set.insert("bad kid", &public_pem(3)).is_err());
        assert!(set.insert("ok", "not a key").is_err());
    }

    #[test]
    fn platform_machine_identity_requires_exact_component_public_key() {
        let mut set = KeySet::empty();
        set.insert(kid::GATEWAY, &public_pem(1)).unwrap();
        assert!(set.resolve(Some(kid::API_GATEWAY)).is_none());
        set.insert(kid::API_GATEWAY, &public_pem(2)).unwrap();
        assert!(set.resolve(Some(kid::API_GATEWAY)).is_some());
        assert!(!set.is_single_key());
    }

    #[test]
    fn json_and_directory_sources_name_components() {
        let json = format!(
            r#"{{"gateway": {}, "hook": {}}}"#,
            serde_json::to_string(&public_pem(4)).unwrap(),
            serde_json::to_string(&public_pem(5)).unwrap()
        );
        let set = KeySet::from_json(&json).unwrap();
        assert_eq!(set.kids(), vec!["gateway", "hook"]);
        assert!(!set.is_single_key());
        assert!(set.resolve(None).is_none(), "no legacy key in the set");
        assert!(KeySet::from_json("{}").is_err());
        assert!(KeySet::from_json("[]").is_err());
        assert!(KeySet::from_json(r#"{"bad kid": "x"}"#).is_err());

        let dir = std::env::temp_dir().join(format!("flare-key-set-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("gateway.pem"), public_pem(6)).unwrap();
        std::fs::write(dir.join("bridge.pub"), public_pem(7)).unwrap();
        std::fs::write(dir.join("README.md"), "ignored").unwrap();
        let set = KeySet::from_dir(&dir).unwrap();
        assert_eq!(set.kids(), vec!["bridge", "gateway"]);
        std::fs::write(dir.join("broken.pem"), "not a key").unwrap();
        assert!(KeySet::from_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(KeySet::from_dir(&dir).is_err());
    }

    #[test]
    fn sources_layer_the_component_set_over_the_legacy_key() {
        assert!(KeySet::from_sources(None, None).unwrap().is_none());
        assert!(
            KeySet::from_sources(Some("  "), Some(""))
                .unwrap()
                .is_none()
        );
        let legacy_only = KeySet::from_sources(None, Some(&public_pem(8)))
            .unwrap()
            .unwrap();
        assert!(legacy_only.is_single_key());
        let json = format!(
            r#"{{"gateway": {}}}"#,
            serde_json::to_string(&public_pem(9)).unwrap()
        );
        let layered = KeySet::from_sources(Some(&json), Some(&public_pem(10)))
            .unwrap()
            .unwrap();
        assert_eq!(layered.kids(), vec!["gateway", LEGACY_KID]);
        assert!(!layered.is_single_key(), "a component set pins kids");
        let set_only = KeySet::from_sources(Some(&json), None).unwrap().unwrap();
        assert_eq!(set_only.kids(), vec!["gateway"]);
        assert!(KeySet::from_sources(Some("/nonexistent/flare-keys"), None).is_err());
    }
}
