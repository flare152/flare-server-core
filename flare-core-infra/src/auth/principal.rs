use std::collections::HashMap;

use async_trait::async_trait;
use thiserror::Error;

use super::token::TokenClaims;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenValidationRequest {
    pub token: String,
    pub trace_id: Option<String>,
    pub request_id: Option<String>,
    pub path: Option<String>,
    pub method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub user_id: String,
    pub tenant_id: Option<String>,
    pub device_id: Option<String>,
    pub app_id: Option<String>,
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
    pub metadata: HashMap<String, String>,
}

impl AuthenticatedPrincipal {
    pub fn from_token_claims(claims: TokenClaims) -> Self {
        Self {
            user_id: claims.sub,
            tenant_id: claims.tenant_id,
            device_id: claims.device_id,
            app_id: None,
            expires_at: Some(claims.exp as i64),
            scopes: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn has_scope(&self, required: &str) -> bool {
        self.scopes
            .iter()
            .any(|scope| scope_matches(scope.as_str(), required))
    }

    pub fn has_any_scope(&self, required: &[&str]) -> bool {
        required.iter().any(|scope| self.has_scope(scope))
    }

    pub fn has_gateway_admin_scope(&self) -> bool {
        self.has_any_scope(&[
            "admin_gateway:admin",
            "admin_gateway:admin:*",
            "core_gateway:admin",
            "core_gateway:admin:*",
            "gateway:admin",
            "flare:admin",
            "admin:*",
            "admin",
        ])
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("authorization token is missing")]
    MissingToken,
    #[error("authorization token is invalid: {0}")]
    InvalidToken(String),
    #[error("authenticated principal is not allowed: {0}")]
    Forbidden(String),
    #[error("auth provider is unavailable: {0}")]
    ProviderUnavailable(String),
}

#[async_trait]
pub trait TokenValidator: Send + Sync {
    async fn validate(
        &self,
        request: TokenValidationRequest,
    ) -> Result<AuthenticatedPrincipal, AuthError>;
}

fn scope_matches(granted: &str, required: &str) -> bool {
    let granted = granted.trim();
    let required = required.trim();
    if granted == "*" || granted == required {
        return true;
    }
    if let Some(prefix) = granted.strip_suffix(":*") {
        return required == prefix || required.starts_with(&format!("{prefix}:"));
    }
    false
}
