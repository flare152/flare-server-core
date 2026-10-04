//! Local API Key policy checks shared by data planes. No Control network access,
//! database ownership, IM/Social scope vocabulary or End User fallback here.
use super::api_key::{ApiKeyKind, ApiKeyPepper, ApiKeyToken};
use ipnet::IpNet;
use std::net::IpAddr;
use subtle::ConstantTimeEq;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApiKeyPolicyError {
    #[error("invalid credential projection")]
    InvalidProjection,
    #[error("API key verification configuration mismatch")]
    PepperMismatch,
    #[error("API key authentication rejected")]
    Unauthenticated,
    #[error("API key authorization rejected")]
    Forbidden,
}

/// Construct only from an authenticated, atomically read local projection.
/// Fields are immutable after construction, including binding and grant.
#[derive(Clone)]
pub struct ApiKeyGrant {
    key_id: String,
    app_id: String,
    service_account_id: String,
    kind: ApiKeyKind,
    digest: [u8; 32],
    pepper_fingerprint: [u8; 32],
    scopes: Vec<String>,
    expires_at_ms: Option<i64>,
    revoked: bool,
    service_account_enabled: bool,
    ip_allow: Vec<IpNet>,
}

impl std::fmt::Debug for ApiKeyGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyGrant")
            .field("key_id", &self.key_id)
            .field("app_id", &self.app_id)
            .field("service_account_id", &self.service_account_id)
            .field("material", &"[REDACTED]")
            .field("revoked", &self.revoked)
            .finish()
    }
}

/// No API Key material or complete grant is returned to downstream consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedApiKey {
    pub key_id: String,
    pub app_id: String,
    pub service_account_id: String,
}

fn identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn scope(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'_' | b'-'))
}

impl ApiKeyGrant {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key_id: String,
        app_id: String,
        service_account_id: String,
        kind: ApiKeyKind,
        digest: [u8; 32],
        pepper_fingerprint: [u8; 32],
        mut scopes: Vec<String>,
        expires_at_ms: Option<i64>,
        revoked: bool,
        service_account_enabled: bool,
        ip_allow: Vec<String>,
    ) -> Result<Self, ApiKeyPolicyError> {
        if key_id.len() != 12
            || !key_id
                .bytes()
                .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
            || !identifier(&app_id, 32)
            || !identifier(&service_account_id, 128)
            || !(1..=16).contains(&scopes.len())
            || scopes.iter().any(|value| !scope(value))
            || expires_at_ms.is_some_and(|value| value <= 0)
            || ip_allow.len() > 32
        {
            return Err(ApiKeyPolicyError::InvalidProjection);
        }
        scopes.sort_unstable();
        if scopes.windows(2).any(|values| values[0] == values[1]) {
            return Err(ApiKeyPolicyError::InvalidProjection);
        }
        let ip_allow = ip_allow
            .into_iter()
            .map(|value| {
                let network = value
                    .parse::<IpNet>()
                    .map_err(|_| ApiKeyPolicyError::InvalidProjection)?;
                // Match PostgreSQL CIDR's canonical network semantics. Never silently
                // widen a malformed host-bit policy or accept an address as a subnet.
                if network.addr() != network.network() || network.to_string() != value {
                    return Err(ApiKeyPolicyError::InvalidProjection);
                }
                Ok(network)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            key_id,
            app_id,
            service_account_id,
            kind,
            digest,
            pepper_fingerprint,
            scopes,
            expires_at_ms,
            revoked,
            service_account_enabled,
            ip_allow,
        })
    }

    /// Call on EVERY request, even on a cache hit. now_ms is UTC milliseconds;
    /// source_ip must be the socket peer or a verified trusted-proxy result.
    /// expected_app=None derives the binding solely from the grant. Some("") or
    /// Some("*") never means default/wildcard. Resource services still do PDP.
    pub fn authorize(
        &self,
        token: &ApiKeyToken,
        pepper: &ApiKeyPepper,
        expected_app: Option<&str>,
        required_scope: &str,
        source_ip: IpAddr,
        now_ms: i64,
    ) -> Result<AuthorizedApiKey, ApiKeyPolicyError> {
        if !bool::from(self.pepper_fingerprint.ct_eq(&pepper.fingerprint())) {
            return Err(ApiKeyPolicyError::PepperMismatch);
        }
        if now_ms <= 0
            || self.revoked
            || !self.service_account_enabled
            || self.expires_at_ms.is_some_and(|expiry| expiry <= now_ms)
            || !token.verify(pepper, self.kind, &self.key_id, &self.digest)
        {
            return Err(ApiKeyPolicyError::Unauthenticated);
        }
        if expected_app.is_some_and(|app| app != self.app_id)
            || !scope(required_scope)
            || self
                .scopes
                .binary_search_by(|value| value.as_str().cmp(required_scope))
                .is_err()
            || (!self.ip_allow.is_empty()
                && !self
                    .ip_allow
                    .iter()
                    .any(|network| network.contains(&source_ip)))
        {
            return Err(ApiKeyPolicyError::Forbidden);
        }
        Ok(AuthorizedApiKey {
            key_id: self.key_id.clone(),
            app_id: self.app_id.clone(),
            service_account_id: self.service_account_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    fn pepper(byte: u8) -> ApiKeyPepper {
        ApiKeyPepper::from_base64url(&URL_SAFE_NO_PAD.encode([byte; 32])).unwrap()
    }
    fn grant(token: &ApiKeyToken, pepper: &ApiKeyPepper) -> ApiKeyGrant {
        ApiKeyGrant::new(
            token.key_id().into(),
            "app-a".into(),
            "sa_a".into(),
            token.kind(),
            token.digest(pepper),
            pepper.fingerprint(),
            vec!["endpoint.read".into(), "endpoint.write".into()],
            Some(2000),
            false,
            true,
            vec![],
        )
        .unwrap()
    }
    fn authorize(
        grant: &ApiKeyGrant,
        token: &ApiKeyToken,
        pepper: &ApiKeyPepper,
        app: Option<&str>,
        permission: &str,
        now: i64,
    ) -> Result<AuthorizedApiKey, ApiKeyPolicyError> {
        grant.authorize(
            token,
            pepper,
            app,
            permission,
            "192.0.2.10".parse().unwrap(),
            now,
        )
    }

    #[test]
    fn identity_comes_only_from_verified_binding_not_payload_or_headers() {
        let pepper = pepper(42);
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let grant = grant(&token, &pepper);
        let identity = authorize(&grant, &token, &pepper, None, "endpoint.read", 1000).unwrap();
        assert_eq!(identity.app_id, "app-a");
        assert_eq!(identity.service_account_id, "sa_a");
        for app in ["", "*", "app-b", " app-a"] {
            assert_eq!(
                authorize(&grant, &token, &pepper, Some(app), "endpoint.read", 1000),
                Err(ApiKeyPolicyError::Forbidden)
            );
        }
        assert!(
            authorize(
                &grant,
                &token,
                &pepper,
                Some("app-a"),
                "endpoint.read",
                1000
            )
            .is_ok()
        );
    }

    #[test]
    fn expiry_and_revocation_apply_on_every_use_without_grace() {
        let pepper = pepper(42);
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let mut grant = grant(&token, &pepper);
        assert!(authorize(&grant, &token, &pepper, None, "endpoint.read", 1999).is_ok());
        for now in [0, -1, 2000, 2001] {
            assert_eq!(
                authorize(&grant, &token, &pepper, None, "endpoint.read", now),
                Err(ApiKeyPolicyError::Unauthenticated)
            );
        }
        grant.revoked = true;
        assert_eq!(
            authorize(&grant, &token, &pepper, None, "endpoint.read", 1000),
            Err(ApiKeyPolicyError::Unauthenticated)
        );
        grant.revoked = false;
        grant.service_account_enabled = false;
        assert_eq!(
            authorize(&grant, &token, &pepper, None, "endpoint.read", 1000),
            Err(ApiKeyPolicyError::Unauthenticated)
        );
    }

    #[test]
    fn scopes_are_exact_and_never_management_or_wildcard_fallback() {
        let pepper = pepper(42);
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let grant = grant(&token, &pepper);
        for permission in [
            "",
            "*",
            "endpoint.*",
            "endpoint.read.child",
            "management.read",
            "endpoint:read",
        ] {
            assert_eq!(
                authorize(&grant, &token, &pepper, None, permission, 1000),
                Err(ApiKeyPolicyError::Forbidden)
            );
        }
    }

    #[test]
    fn wrong_secret_selector_kind_and_pepper_fail_closed() {
        let pepper_a = pepper(42);
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let grant = grant(&token, &pepper_a);
        let encoded = token.into_issuance_token();
        let wrong_kind = ApiKeyToken::parse(&encoded.replacen("flr_live", "flr_test", 1)).unwrap();
        assert_eq!(
            authorize(&grant, &wrong_kind, &pepper_a, None, "endpoint.read", 1000),
            Err(ApiKeyPolicyError::Unauthenticated)
        );
        let other = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let other_encoded = other.into_issuance_token();
        let wrong_secret = ApiKeyToken::parse(&format!(
            "flr_live_{}_{}",
            grant.key_id,
            other_encoded.splitn(4, '_').nth(3).unwrap()
        ))
        .unwrap();
        assert_eq!(
            authorize(
                &grant,
                &wrong_secret,
                &pepper_a,
                None,
                "endpoint.read",
                1000
            ),
            Err(ApiKeyPolicyError::Unauthenticated)
        );
        let token = ApiKeyToken::parse(&encoded).unwrap();
        assert_eq!(
            authorize(&grant, &token, &pepper(43), None, "endpoint.read", 1000),
            Err(ApiKeyPolicyError::PepperMismatch)
        );
    }

    #[test]
    fn cidr_matches_v4_v6_but_not_mapped_v4_or_an_untrusted_address() {
        let pepper = pepper(42);
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let mut grant = grant(&token, &pepper);
        grant.ip_allow = vec![
            "192.0.2.0/24".parse().unwrap(),
            "2001:db8::/32".parse().unwrap(),
        ];
        for source in ["192.0.2.10", "2001:db8::1"] {
            assert!(
                grant
                    .authorize(
                        &token,
                        &pepper,
                        None,
                        "endpoint.read",
                        source.parse().unwrap(),
                        1000
                    )
                    .is_ok()
            );
        }
        // Do not invent forwarded-IP or mapped-address normalization here.
        for source in ["198.51.100.1", "2001:db9::1", "::ffff:192.0.2.10"] {
            assert_eq!(
                grant.authorize(
                    &token,
                    &pepper,
                    None,
                    "endpoint.read",
                    source.parse().unwrap(),
                    1000
                ),
                Err(ApiKeyPolicyError::Forbidden)
            );
        }
    }

    #[test]
    fn projection_validation_is_bounded_and_debug_redacts_material() {
        let pepper = pepper(42);
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let make = |app: &str, scopes: Vec<String>, ips: Vec<String>, expiry| {
            ApiKeyGrant::new(
                token.key_id().into(),
                app.into(),
                "sa_a".into(),
                token.kind(),
                token.digest(&pepper),
                pepper.fingerprint(),
                scopes,
                expiry,
                false,
                true,
                ips,
            )
        };
        for app in ["", "*", " app-a", "app/a", "租户"] {
            assert!(make(app, vec!["read".into()], vec![], None).is_err());
        }
        for scopes in [
            vec![],
            vec!["*".into()],
            vec!["read".into(), "read".into()],
            vec!["read".into(); 17],
            vec!["a b".into()],
        ] {
            assert!(make("app", scopes, vec![], None).is_err());
        }
        for ips in [
            vec!["192.0.2.1/24".into()],
            vec!["192.0.2.1".into()],
            vec!["bad".into()],
            vec!["192.0.2.0/24".into(); 33],
        ] {
            assert!(make("app", vec!["read".into()], ips, None).is_err());
        }
        assert!(make("app", vec!["read".into()], vec![], Some(0)).is_err());
        let grant = make(
            "app",
            vec!["read".into()],
            vec!["192.0.2.0/24".into()],
            None,
        )
        .unwrap();
        assert!(format!("{grant:?}").contains("[REDACTED]"));
        assert!(!format!("{grant:?}").contains(&format!("{:?}", token.digest(&pepper))));
    }
}
