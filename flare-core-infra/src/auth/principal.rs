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
            // HS256 令牌没有 scope 声明;管理用途由 `token_use = "admin"` 表达,
            // 在这里派生成 `admin:*`,否则 admin-gateway 对本地签发的任何令牌都是 403。
            scopes: if claims.token_use.as_deref() == Some(super::token::TOKEN_USE_ADMIN) {
                vec!["admin:*".to_string()]
            } else {
                Vec::new()
            },
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

/// [`AuthError::InvalidToken`] 里表示「签名与签发方都对，只是已过期」的类型化原因前缀。
pub const TOKEN_EXPIRED_REASON: &str = "TOKEN_EXPIRED";

impl AuthError {
    /// 令牌签名与签发方都对、只是已过期：这是对令牌的确定结论，客户端应换一枚新令牌。
    ///
    /// 仍是 [`AuthError::InvalidToken`]（不新增变体，免得破坏调用方对本枚举的穷举匹配），
    /// 说明以 [`TOKEN_EXPIRED_REASON`] 开头，用 [`Self::is_expired_token`] 识别。
    pub fn expired_token(detail: impl std::fmt::Display) -> Self {
        AuthError::InvalidToken(format!("{TOKEN_EXPIRED_REASON}: {detail}"))
    }

    /// 是否为 [`Self::expired_token`] 给出的过期结论。
    pub fn is_expired_token(&self) -> bool {
        matches!(
            self,
            AuthError::InvalidToken(detail)
                if detail
                    .strip_prefix(TOKEN_EXPIRED_REASON)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
        )
    }
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

#[cfg(test)]
mod token_use_tests {
    use super::*;

    fn claims(token_use: Option<&str>) -> TokenClaims {
        TokenClaims {
            sub: "emp".into(),
            iss: "x".into(),
            exp: 1,
            iat: 0,
            jti: "j".into(),
            device_id: None,
            tenant_id: Some("0".into()),
            token_use: token_use.map(str::to_string),
        }
    }

    #[test]
    fn admin_token_use_derives_admin_scope() {
        assert!(
            AuthenticatedPrincipal::from_token_claims(claims(Some("admin")))
                .has_gateway_admin_scope()
        );
        assert!(!AuthenticatedPrincipal::from_token_claims(claims(None)).has_gateway_admin_scope());
        assert!(
            !AuthenticatedPrincipal::from_token_claims(claims(Some("control")))
                .has_gateway_admin_scope()
        );
    }
}
