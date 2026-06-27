use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::{StatusCode, header::HeaderName};
use serde::{Deserialize, Serialize};

use super::{
    AuthError, AuthenticatedPrincipal, CompositeTokenValidator, TokenService,
    TokenValidationRequest, TokenValidator,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthProviderMode {
    #[default]
    CoreJwt,
    HttpHook,
}

impl std::str::FromStr for AuthProviderMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "core_jwt" => Ok(Self::CoreJwt),
            "http_hook" => Ok(Self::HttpHook),
            other => anyhow::bail!("unsupported auth provider mode: {other}"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthProviderConfig {
    #[serde(default)]
    pub mode: AuthProviderMode,
    #[serde(default)]
    pub hook_url: Option<String>,
    #[serde(default = "default_auth_hook_timeout_ms")]
    pub hook_timeout_ms: u64,
    #[serde(default = "default_auth_hook_secret_header")]
    pub hook_secret_header: String,
    #[serde(default)]
    pub hook_secret: Option<String>,
}

impl Default for AuthProviderConfig {
    fn default() -> Self {
        Self {
            mode: AuthProviderMode::CoreJwt,
            hook_url: None,
            hook_timeout_ms: default_auth_hook_timeout_ms(),
            hook_secret_header: default_auth_hook_secret_header(),
            hook_secret: None,
        }
    }
}

fn default_auth_hook_timeout_ms() -> u64 {
    800
}

fn default_auth_hook_secret_header() -> String {
    "x-flare-auth-hook-secret".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrustedIssuerConfig {
    pub issuer: String,
    pub secret: String,
}

#[derive(Clone)]
pub struct CoreJwtTokenValidator {
    validator: CompositeTokenValidator,
}

impl CoreJwtTokenValidator {
    pub fn new(validator: CompositeTokenValidator) -> Self {
        Self { validator }
    }
}

#[async_trait]
impl TokenValidator for CoreJwtTokenValidator {
    async fn validate(
        &self,
        request: TokenValidationRequest,
    ) -> std::result::Result<AuthenticatedPrincipal, AuthError> {
        self.validator
            .validate_token(&request.token)
            .map(AuthenticatedPrincipal::from_token_claims)
            .map_err(|err| AuthError::InvalidToken(err.to_string()))
    }
}

#[derive(Clone)]
pub struct HttpHookTokenValidator {
    client: reqwest::Client,
    endpoint: String,
    secret_header: HeaderName,
    secret: Option<String>,
}

impl HttpHookTokenValidator {
    pub fn new(config: &AuthProviderConfig) -> Result<Self> {
        let endpoint = config
            .hook_url
            .clone()
            .context("auth hook URL is required for http_hook auth mode")?;
        let secret_header = HeaderName::from_bytes(config.hook_secret_header.as_bytes())
            .context("auth hook secret header is not a valid HTTP header name")?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.hook_timeout_ms))
            .build()
            .context("failed to build auth hook HTTP client")?;

        Ok(Self {
            client,
            endpoint,
            secret_header,
            secret: config.hook_secret.clone(),
        })
    }
}

#[async_trait]
impl TokenValidator for HttpHookTokenValidator {
    async fn validate(
        &self,
        request: TokenValidationRequest,
    ) -> std::result::Result<AuthenticatedPrincipal, AuthError> {
        let mut http_request = self
            .client
            .post(&self.endpoint)
            .json(&HookValidationRequest {
                token: request.token,
                trace_id: request.trace_id,
                request_id: request.request_id,
                path: request.path,
                method: request.method,
            });

        if let Some(secret) = self.secret.as_deref() {
            http_request = http_request.header(self.secret_header.clone(), secret);
        }

        let response = http_request
            .send()
            .await
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
        let status = response.status();

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(AuthError::InvalidToken(format!(
                "auth hook rejected token with status {status}"
            )));
        }
        if !status.is_success() {
            return Err(AuthError::ProviderUnavailable(format!(
                "auth hook returned status {status}"
            )));
        }

        response
            .json::<HookValidationResponse>()
            .await
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?
            .into_principal()
    }
}

#[derive(Debug, Serialize)]
struct HookValidationRequest {
    token: String,
    trace_id: Option<String>,
    request_id: Option<String>,
    path: Option<String>,
    method: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HookValidationResponse {
    active: bool,
    user_id: Option<String>,
    tenant_id: Option<String>,
    device_id: Option<String>,
    app_id: Option<String>,
    expires_at: Option<i64>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

impl HookValidationResponse {
    fn into_principal(self) -> std::result::Result<AuthenticatedPrincipal, AuthError> {
        if !self.active {
            return Err(AuthError::InvalidToken(
                "auth hook returned inactive token".to_string(),
            ));
        }

        let user_id = self
            .user_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AuthError::ProviderUnavailable(
                    "auth hook returned active principal without user_id".to_string(),
                )
            })?;

        Ok(AuthenticatedPrincipal {
            user_id,
            tenant_id: self.tenant_id.filter(|value| !value.trim().is_empty()),
            device_id: self.device_id.filter(|value| !value.trim().is_empty()),
            app_id: self.app_id.filter(|value| !value.trim().is_empty()),
            expires_at: self.expires_at,
            scopes: self.scopes,
            metadata: self.metadata,
        })
    }
}

pub fn build_token_validator(
    config: &AuthProviderConfig,
    token_service: Arc<TokenService>,
    trusted_issuers: &[TrustedIssuerConfig],
) -> Result<Arc<dyn TokenValidator>> {
    match config.mode {
        AuthProviderMode::CoreJwt => build_core_jwt_token_validator(token_service, trusted_issuers),
        AuthProviderMode::HttpHook => build_http_hook_token_validator(config),
    }
}

pub fn build_core_jwt_token_validator(
    token_service: Arc<TokenService>,
    trusted_issuers: &[TrustedIssuerConfig],
) -> Result<Arc<dyn TokenValidator>> {
    let mut validator = CompositeTokenValidator::new(token_service);
    for trusted in trusted_issuers {
        let issuer = trusted.issuer.trim();
        let secret = trusted.secret.trim();
        if issuer.is_empty() || secret.is_empty() {
            anyhow::bail!("trusted token issuer and secret cannot be empty");
        }
        validator.push_trusted_issuer(secret.to_string(), issuer.to_string());
    }
    Ok(Arc::new(CoreJwtTokenValidator::new(validator)))
}

pub fn build_http_hook_token_validator(
    config: &AuthProviderConfig,
) -> Result<Arc<dyn TokenValidator>> {
    Ok(Arc::new(HttpHookTokenValidator::new(config)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_response_maps_active_principal() {
        let response = HookValidationResponse {
            active: true,
            user_id: Some(" user-a ".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            device_id: Some("device-a".to_string()),
            app_id: Some("business-console".to_string()),
            expires_at: Some(100),
            scopes: vec!["message:send".to_string()],
            metadata: HashMap::from([("source".to_string(), "business".to_string())]),
        };

        let principal = response.into_principal().expect("active principal");

        assert_eq!(principal.user_id, "user-a");
        assert_eq!(principal.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(principal.device_id.as_deref(), Some("device-a"));
        assert_eq!(principal.app_id.as_deref(), Some("business-console"));
        assert_eq!(principal.scopes, vec!["message:send"]);
        assert_eq!(
            principal.metadata.get("source").map(String::as_str),
            Some("business")
        );
    }

    #[tokio::test]
    async fn core_jwt_validator_accepts_core_token() {
        let token_service = Arc::new(TokenService::new("secret", "flare-im-core", 3600));
        let token = token_service
            .generate_token("user-a", Some("device-a"), Some("tenant-a"))
            .expect("token");
        let validator = CoreJwtTokenValidator::new(CompositeTokenValidator::new(token_service));

        let principal = validator
            .validate(TokenValidationRequest {
                token,
                trace_id: None,
                request_id: None,
                path: None,
                method: None,
            })
            .await
            .expect("principal");

        assert_eq!(principal.user_id, "user-a");
        assert_eq!(principal.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(principal.device_id.as_deref(), Some("device-a"));
    }
}
