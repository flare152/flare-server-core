use std::sync::Arc;

use anyhow::{Result, anyhow};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};

use super::token::{TokenClaims, TokenService};

/// 额外信任的 JWT 发行方（如 Social 登录签发的 access_token）。
#[derive(Debug, Clone)]
pub struct TrustedIssuer {
    issuer: String,
    decoding_key: DecodingKey,
}

impl TrustedIssuer {
    pub fn new(secret: impl Into<String>, issuer: impl Into<String>) -> Self {
        let secret = secret.into();
        let issuer = issuer.into();
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());
        Self {
            issuer,
            decoding_key,
        }
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn validate(&self, token: &str) -> Result<TokenClaims> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        decode::<TokenClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|err| anyhow!("invalid token for issuer {}: {err}", self.issuer))
    }
}

/// 主 TokenService + 可选额外 issuer，用于 IM 长连接等多来源 JWT 校验。
#[derive(Clone)]
pub struct CompositeTokenValidator {
    primary: Arc<TokenService>,
    trusted: Vec<TrustedIssuer>,
}

impl CompositeTokenValidator {
    pub fn new(primary: Arc<TokenService>) -> Self {
        Self {
            primary,
            trusted: Vec::new(),
        }
    }

    pub fn with_trusted_issuer(
        mut self,
        secret: impl Into<String>,
        issuer: impl Into<String>,
    ) -> Self {
        self.trusted.push(TrustedIssuer::new(secret, issuer));
        self
    }

    pub fn push_trusted_issuer(&mut self, secret: impl Into<String>, issuer: impl Into<String>) {
        self.trusted.push(TrustedIssuer::new(secret, issuer));
    }

    pub fn validate_token(&self, token: &str) -> Result<TokenClaims> {
        if let Ok(claims) = self.primary.validate_token(token) {
            return Ok(claims);
        }
        let mut last_err: Option<anyhow::Error> = None;
        for trusted in &self.trusted {
            match trusted.validate(token) {
                Ok(claims) => return Ok(claims),
                Err(err) => last_err = Some(err),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("token validation failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_accepts_primary_and_trusted_issuer() {
        let primary = Arc::new(TokenService::new("im-secret", "flare-im-core", 3600));
        let composite = CompositeTokenValidator::new(primary.clone())
            .with_trusted_issuer("social-secret", "flare-social");

        let im_token = primary
            .generate_token("usr_a", None, Some("default"))
            .expect("im token");
        let social = TokenService::new("social-secret", "flare-social", 3600);
        let social_token = social
            .generate_token("usr_b", None, Some("default"))
            .expect("social token");

        assert_eq!(
            composite.validate_token(&im_token).expect("im").sub,
            "usr_a"
        );
        assert_eq!(
            composite.validate_token(&social_token).expect("social").sub,
            "usr_b"
        );
    }
}
