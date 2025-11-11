use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::store::TokenStore;

/// JWT claims used for access tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    pub sub: String,
    pub iss: String,
    pub exp: usize,
    pub iat: usize,
    pub jti: String,
    pub device_id: Option<String>,
    pub tenant_id: Option<String>,
}

/// Stateless token service backed by HMAC (HS256)
pub struct TokenService {
    secret: String,
    issuer: String,
    ttl: Duration,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    store: Option<Arc<dyn TokenStore>>,
}

impl TokenService {
    /// Creates a new token service
    pub fn new(secret: impl Into<String>, issuer: impl Into<String>, ttl_seconds: u64) -> Self {
        let secret = secret.into();
        let issuer = issuer.into();
        let encoding_key = EncodingKey::from_secret(secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());
        Self {
            secret,
            issuer,
            ttl: Duration::from_secs(ttl_seconds.max(60)),
            encoding_key,
            decoding_key,
            store: None,
        }
    }

    /// Attach an external token store (e.g. Redis)
    pub fn with_store(mut self, store: Arc<dyn TokenStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Issues a token for the given user/device/tenant
    pub fn generate_token(
        &self,
        user_id: &str,
        device_id: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| anyhow!("system time error: {err}"))?;
        let iat = now.as_secs() as usize;
        let exp = (now + self.ttl).as_secs() as usize;

        let claims = TokenClaims {
            sub: user_id.to_string(),
            iss: self.issuer.clone(),
            exp,
            iat,
            jti: Uuid::new_v4().to_string(),
            device_id: device_id.map(|v| v.to_string()),
            tenant_id: tenant_id.map(|v| v.to_string()),
        };

        let token = encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|err| anyhow!("failed to encode token: {err}"))?;

        if let Some(store) = self.store.as_ref() {
            store.register(&claims.sub, &claims.jti, self.ttl)?;
        }

        Ok(token)
    }

    /// Validates the token and returns the decoded claims
    pub fn validate_token(&self, token: &str) -> Result<TokenClaims> {
        let claims = self.decode_claims(token)?;

        if let Some(store) = self.store.as_ref() {
            if store.is_revoked(&claims.jti)? {
                return Err(anyhow!("token revoked"));
            }
        }

        Ok(claims)
    }

    /// Refreshes the token (issues a new one and revokes the old token)
    pub fn refresh_token(&self, token: &str) -> Result<String> {
        let claims = self.validate_token(token)?; // validates revocation as well
        let new_token = self.generate_token(
            &claims.sub,
            claims.device_id.as_deref(),
            claims.tenant_id.as_deref(),
        )?;

        if let Some(store) = self.store.as_ref() {
            let remaining_secs = claims.exp.saturating_sub(current_epoch_seconds());
            let ttl = Duration::from_secs(remaining_secs.max(1) as u64);
            store.revoke(&claims.sub, &claims.jti, ttl)?;
        }

        Ok(new_token)
    }

    /// Revokes the provided token (requires token store)
    pub fn revoke_token(&self, token: &str) -> Result<()> {
        let claims = self.decode_claims(token)?;

        if let Some(store) = self.store.as_ref() {
            let remaining_secs = claims.exp.saturating_sub(current_epoch_seconds());
            let ttl = Duration::from_secs(remaining_secs.max(1) as u64);
            store.revoke(&claims.sub, &claims.jti, ttl)?;
        }

        Ok(())
    }

    /// Revokes all tokens for a user (requires token store)
    pub fn revoke_user(&self, user_id: &str) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store.revoke_user(user_id, self.ttl)?;
        }
        Ok(())
    }

    /// Returns token TTL
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Returns issuer
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Returns the secret (for compatibility / debugging)
    pub fn secret(&self) -> &str {
        &self.secret
    }

    fn decode_claims(&self, token: &str) -> Result<TokenClaims> {
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_issuer(&[self.issuer.as_str()]);

        decode::<TokenClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|err| anyhow!("invalid token: {err}"))
    }
}

impl TokenClaims {
    pub fn issued_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(self.iat as i64, 0).unwrap_or_else(Utc::now)
    }

    pub fn expires_at(&self) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(self.exp as i64, 0).unwrap_or_else(Utc::now)
    }
}

fn current_epoch_seconds() -> usize {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as usize)
        .unwrap_or(0)
}
