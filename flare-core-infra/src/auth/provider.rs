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
    /// `http_hook` 模式下签发/刷新转发地址；未配置则签发接口返回 501。
    #[serde(default)]
    pub hook_issue_url: Option<String>,
    /// 联调开关：允许不带 app 凭据、直接按 userId 签发。默认关；开启等于任何人可冒任意身份。
    #[serde(default)]
    pub dev_issue: bool,
    /// 业务后端签发凭据（服务端到服务端）。客户端永远不持有。
    #[serde(default)]
    pub app_credentials: Vec<AppCredential>,
    /// 刷新宽限：token 过期后多久内仍可换新。
    #[serde(default = "default_refresh_grace_secs")]
    pub refresh_grace_secs: u64,
}

impl Default for AuthProviderConfig {
    fn default() -> Self {
        Self {
            mode: AuthProviderMode::CoreJwt,
            hook_url: None,
            hook_timeout_ms: default_auth_hook_timeout_ms(),
            hook_secret_header: default_auth_hook_secret_header(),
            hook_secret: None,
            hook_issue_url: None,
            dev_issue: false,
            app_credentials: Vec::new(),
            refresh_grace_secs: default_refresh_grace_secs(),
        }
    }
}

fn default_refresh_grace_secs() -> u64 {
    7 * 24 * 3600
}

/// 业务后端调用签发接口的凭据：`x-app-id` + `x-app-secret`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AppCredential {
    pub app_id: String,
    /// Authoritative data-plane tenant. Never supplied by the token issuance caller.
    pub tenant_id: String,
    pub secret: String,
}

impl AppCredential {
    /// 解析 `app1:tenant1:secret1,app2:tenant2:secret2` 形式的环境变量。
    pub fn parse_list(value: &str) -> Result<Vec<Self>> {
        let mut out = Vec::new();
        for item in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let mut fields = item.splitn(3, ':');
            let (Some(app_id), Some(tenant_id), Some(secret)) =
                (fields.next(), fields.next(), fields.next())
            else {
                // Never include input: a malformed entry may itself contain the secret.
                anyhow::bail!("app credential must be `app_id:tenant_id:secret`");
            };
            let (app_id, tenant_id, secret) = (app_id.trim(), tenant_id.trim(), secret.trim());
            if app_id.is_empty() || tenant_id.is_empty() || secret.len() < 16 {
                anyhow::bail!(
                    "app credential needs non-empty app_id, tenant_id and a secret of at least 16 bytes"
                );
            }
            if out.iter().any(|cred: &Self| cred.app_id == app_id) {
                anyhow::bail!("duplicate app credential id");
            }
            out.push(Self {
                app_id: app_id.to_string(),
                tenant_id: tenant_id.to_string(),
                secret: secret.to_string(),
            });
        }
        Ok(out)
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
        let claims = match self.validator.validate_token(&request.token) {
            Ok(claims) => claims,
            // 签名与签发方都对、只是过期了：给出可识别的过期结论，链式校验据此不让「hook 不可用」
            // 盖掉它，客户端会去换令牌，而不是带着过期令牌一直退避重连。
            Err(err) if self.validator.is_expired(&request.token) => {
                return Err(AuthError::expired_token(err));
            }
            Err(err) => return Err(AuthError::InvalidToken(err.to_string())),
        };
        // This validator protects normal user API routes. A refresh or control
        // token has a valid signature but must never become a User principal.
        // Legacy access tokens omit token_use; preserve that documented case.
        if !matches!(claims.token_use.as_deref(), None | Some("access")) {
            // Forbidden is terminal in ChainedTokenValidator: do not forward a
            // locally verified refresh/control token to an opaque-token Hook.
            return Err(AuthError::Forbidden("token is not an access token".into()));
        }
        Ok(AuthenticatedPrincipal::from_token_claims(claims))
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
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_millis(config.hook_timeout_ms))
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

        let mut response = http_request
            .send()
            .await
            .map_err(|_| AuthError::ProviderUnavailable("auth hook request failed".into()))?;
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

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AuthError::ProviderUnavailable("auth hook response failed".into()))?
        {
            if body.len() + chunk.len() > 8192 {
                return Err(AuthError::ProviderUnavailable(
                    "auth hook response too large".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice::<HookValidationResponse>(&body)
            .map_err(|_| AuthError::ProviderUnavailable("invalid auth hook response".into()))?
            .into_principal()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookValidationRequest {
    token: String,
    trace_id: Option<String>,
    request_id: Option<String>,
    path: Option<String>,
    method: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookValidationResponse {
    active: bool,
    reason: Option<String>,
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
            if self.reason.as_deref() == Some(super::TOKEN_EXPIRED_REASON) {
                return Err(AuthError::expired_token("auth hook verified expiration"));
            }
            return Err(AuthError::InvalidToken(
                "auth hook returned inactive token".to_string(),
            ));
        }

        let user_id = self
            .user_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            // hook 认了这枚令牌却给不出用户：这是 hook 的结论（或契约错误），不是暂时不可用。
            // 报成 ProviderUnavailable 会让客户端带着同一枚令牌无限退避重连。
            .ok_or_else(|| {
                AuthError::Forbidden(
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

/// 分层校验器：按顺序尝试每一层，第一层通过即返回。
///
/// 典型编排 = [本地(core JWT + 受信 issuer), 三方验证 hook]：core 自签 / 受信 issuer 的
/// token 走本地零网络开销；本地都不认的（三方 opaque token）才回落到 hook 询问业务方。
/// 这样一套部署**既能 core 独立完成验证，又能适配三方业务的 token 验证**。
///
/// 错误合并策略：
/// - 某层 `Forbidden`/`MissingToken`：立即返回（身份已识别但不允许 / 压根没 token，无需再试）。
/// - 某层 `InvalidToken`：这层不认这枚 token，继续下一层。
/// - 某层 `ProviderUnavailable`：这层（多为 hook）暂时不可用，记下后继续下一层。
/// - 全部失败：有层给出过过期结论（[`AuthError::is_expired_token`]，本地层验过签名、只是过期）就
///   返回它，客户端据此换令牌；否则只要有过 `ProviderUnavailable` 就返回它（可重试，别把「没验成」
///   误判成「非法」把可能合法的三方 token 永久拒掉）；否则返回最后一次 `InvalidToken`。以前过期结论
///   也被 `ProviderUnavailable` 盖掉，hook 一挂，客户端就带着过期令牌一直退避重连、不去刷新。
#[derive(Clone)]
pub struct ChainedTokenValidator {
    layers: Vec<Arc<dyn TokenValidator>>,
}

impl ChainedTokenValidator {
    pub fn new(layers: Vec<Arc<dyn TokenValidator>>) -> Self {
        Self { layers }
    }
}

#[async_trait]
impl TokenValidator for ChainedTokenValidator {
    async fn validate(
        &self,
        request: TokenValidationRequest,
    ) -> std::result::Result<AuthenticatedPrincipal, AuthError> {
        let mut expired: Option<AuthError> = None;
        let mut last_invalid: Option<AuthError> = None;
        let mut had_unavailable: Option<AuthError> = None;
        for layer in &self.layers {
            match layer.validate(request.clone()).await {
                Ok(principal) => return Ok(principal),
                Err(err @ (AuthError::Forbidden(_) | AuthError::MissingToken)) => return Err(err),
                Err(err @ AuthError::InvalidToken(_)) if err.is_expired_token() => {
                    expired.get_or_insert(err);
                }
                Err(err @ AuthError::InvalidToken(_)) => last_invalid = Some(err),
                Err(err @ AuthError::ProviderUnavailable(_)) => had_unavailable = Some(err),
            }
        }
        Err(expired
            .or(had_unavailable)
            .or(last_invalid)
            .unwrap_or_else(|| {
                AuthError::InvalidToken("no validator accepted the token".to_string())
            }))
    }
}

pub fn build_token_validator(
    config: &AuthProviderConfig,
    token_service: Arc<TokenService>,
    trusted_issuers: &[TrustedIssuerConfig],
) -> Result<Arc<dyn TokenValidator>> {
    match config.mode {
        // core_jwt：本地校验（core JWT + 受信 issuer）。若同时配了 hook_url，则把三方验证 hook
        // 作为**兜底层**接在本地之后——本地认得的零网络开销，认不得的才问业务方。
        // 这就是「既能 core 独立验证，又能适配三方 token 验证」的混合形态。
        AuthProviderMode::CoreJwt => {
            let local = build_core_jwt_token_validator(token_service, trusted_issuers)?;
            match config
                .hook_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(_) => {
                    let hook = build_http_hook_token_validator(config)?;
                    Ok(Arc::new(ChainedTokenValidator::new(vec![local, hook])))
                }
                None => Ok(local),
            }
        }
        // http_hook：业务方拥有整套 token 流程，core 只把每枚 token 交给 hook 验证（纯委托）。
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
    use super::super::{TOKEN_EXPIRED_REASON, TokenClaims};
    use super::*;
    #[test]
    fn hook_expiration_is_a_closed_reason_not_an_arbitrary_retry_hint() {
        let expired: HookValidationResponse =
            serde_json::from_value(serde_json::json!({"active":false,"reason":"TOKEN_EXPIRED"}))
                .unwrap();
        assert!(expired.into_principal().unwrap_err().is_expired_token());
        let rejected: HookValidationResponse = serde_json::from_value(
            serde_json::json!({"active":false,"reason":"please retry TOKEN_EXPIRED"}),
        )
        .unwrap();
        assert!(!rejected.into_principal().unwrap_err().is_expired_token());
    }

    #[tokio::test]
    async fn http_hook_never_follows_redirect_or_consumes_unbounded_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = target.local_addr().unwrap();
        for oversized in [false, true] {
            let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/auth", source.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = source.accept().await.unwrap();
                let mut input = [0; 8192];
                let _ = socket.read(&mut input).await.unwrap();
                let response = if oversized {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: 8193\r\nConnection: close\r\n\r\n{}",
                        "x".repeat(8193)
                    )
                } else {
                    format!(
                        "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{destination}/must-not-receive-secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
            });
            let validator = HttpHookTokenValidator::new(&AuthProviderConfig {
                hook_url: Some(endpoint),
                hook_secret: Some("fixture-only-internal-transport-credential".into()),
                hook_timeout_ms: 1000,
                ..Default::default()
            })
            .unwrap();
            let result = validator
                .validate(TokenValidationRequest {
                    token: "not-a-real-token".into(),
                    trace_id: None,
                    request_id: None,
                    path: None,
                    method: None,
                })
                .await;
            assert!(matches!(result, Err(AuthError::ProviderUnavailable(_))));
            server.await.unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), target.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn hook_response_maps_active_principal() {
        let response = HookValidationResponse {
            active: true,
            reason: None,
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

    #[derive(Clone)]
    struct StubValidator {
        // token 值 -> 结果（Ok(user_id) / Err）
        accepts: Option<String>,
        result: std::sync::Arc<std::sync::Mutex<Option<AuthError>>>,
    }

    #[async_trait]
    impl TokenValidator for StubValidator {
        async fn validate(
            &self,
            request: TokenValidationRequest,
        ) -> std::result::Result<AuthenticatedPrincipal, AuthError> {
            if let Some(err) = self.result.lock().unwrap().take() {
                return Err(err);
            }
            if self.accepts.as_deref() == Some(request.token.as_str()) {
                return Ok(AuthenticatedPrincipal {
                    user_id: format!("u-{}", request.token),
                    tenant_id: None,
                    device_id: None,
                    app_id: None,
                    expires_at: None,
                    scopes: Vec::new(),
                    metadata: HashMap::new(),
                });
            }
            Err(AuthError::InvalidToken("stub: not my token".into()))
        }
    }

    fn stub(accepts: &str) -> Arc<dyn TokenValidator> {
        Arc::new(StubValidator {
            accepts: Some(accepts.to_string()),
            result: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
    }

    fn req(token: &str) -> TokenValidationRequest {
        TokenValidationRequest {
            token: token.into(),
            trace_id: None,
            request_id: None,
            path: None,
            method: None,
        }
    }

    #[test]
    fn hook_response_deserializes_camelcase() {
        // 验证 hook 响应契约与鉴权 API 其余部分一致，走 camelCase。
        let raw = r#"{"active":true,"userId":"3p-alice","tenantId":"0","expiresAt":123,"scopes":["message:send"],"metadata":{"source":"biz"}}"#;
        let resp: HookValidationResponse = serde_json::from_str(raw).unwrap();
        let p = resp.into_principal().unwrap();
        assert_eq!(p.user_id, "3p-alice");
        assert_eq!(p.tenant_id.as_deref(), Some("0"));
        assert_eq!(p.expires_at, Some(123));
        assert_eq!(p.scopes, vec!["message:send"]);
    }

    #[tokio::test]
    async fn chained_validates_local_first_then_hook_fallback() {
        // 层1=本地(认 "core-tok")，层2=hook(认 "3p-tok")
        let chain = ChainedTokenValidator::new(vec![stub("core-tok"), stub("3p-tok")]);
        assert_eq!(
            chain.validate(req("core-tok")).await.unwrap().user_id,
            "u-core-tok"
        );
        assert_eq!(
            chain.validate(req("3p-tok")).await.unwrap().user_id,
            "u-3p-tok"
        );
        // 两层都不认 -> InvalidToken
        assert!(matches!(
            chain.validate(req("nope")).await.unwrap_err(),
            AuthError::InvalidToken(_)
        ));
    }

    #[tokio::test]
    async fn chained_hook_unavailable_is_retryable_not_permanent_reject() {
        // 层1 本地不认（InvalidToken），层2 hook 不可用（ProviderUnavailable）：
        // 合并后应为 ProviderUnavailable（可重试），而不是把可能合法的三方 token 永久拒掉。
        let local = stub("core-tok");
        let down = Arc::new(StubValidator {
            accepts: None,
            result: std::sync::Arc::new(std::sync::Mutex::new(Some(
                AuthError::ProviderUnavailable("hook down".into()),
            ))),
        });
        let chain = ChainedTokenValidator::new(vec![local, down]);
        assert!(matches!(
            chain.validate(req("3p-tok")).await.unwrap_err(),
            AuthError::ProviderUnavailable(_)
        ));
    }

    /// 每次都不可用的 hook（StubValidator 的预置错误只用一次）。
    struct DownHook;

    #[async_trait]
    impl TokenValidator for DownHook {
        async fn validate(
            &self,
            _request: TokenValidationRequest,
        ) -> std::result::Result<AuthenticatedPrincipal, AuthError> {
            Err(AuthError::ProviderUnavailable("hook down".into()))
        }
    }

    fn unavailable_hook() -> Arc<dyn TokenValidator> {
        Arc::new(DownHook)
    }

    /// 本地签发、签名有效但已过期 100 s 的令牌（超过 jsonwebtoken 默认 60 s 的容差）。
    fn expired_core_token(secret: &str) -> String {
        use jsonwebtoken::{EncodingKey, Header, encode};
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize;
        encode(
            &Header::default(),
            &TokenClaims {
                sub: "user-a".into(),
                iss: "flare-im-core".into(),
                exp: now - 100,
                iat: now - 200,
                jti: "j-expired".into(),
                device_id: None,
                tenant_id: Some("tenant-a".into()),
                token_use: None,
            },
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    /// 本地层验过签名、判定过期，hook 此时不可用：保留过期结论（客户端去换令牌），不能被
    /// ProviderUnavailable 盖掉——那样客户端会带着过期令牌一直退避重连。
    #[tokio::test]
    async fn chained_keeps_the_local_expiry_verdict_when_the_hook_is_down() {
        let secret = "a-strong-shared-secret-with-more-than-32-bytes!";
        let token_service = Arc::new(TokenService::new(secret, "flare-im-core", 3600));
        let local = Arc::new(CoreJwtTokenValidator::new(CompositeTokenValidator::new(
            token_service,
        )));
        let expired = expired_core_token(secret);

        let err = local.validate(req(&expired)).await.unwrap_err();
        assert!(err.is_expired_token(), "{err}");

        let chain = ChainedTokenValidator::new(vec![local, unavailable_hook()]);
        let err = chain.validate(req(&expired)).await.unwrap_err();
        assert!(
            matches!(&err, AuthError::InvalidToken(_)) && err.is_expired_token(),
            "{err}"
        );

        // 本地不认（别家签名的令牌）且 hook 不可用：仍是可重试的 ProviderUnavailable
        let foreign = expired_core_token("another-secret-with-more-than-32-bytes-long!!");
        let err = chain.validate(req(&foreign)).await.unwrap_err();
        assert!(matches!(err, AuthError::ProviderUnavailable(_)), "{err}");
    }

    /// 过期结论只认签名有效的本地令牌：被篡改、乱码、别家签名的都不是「过期」。
    #[tokio::test]
    async fn only_signature_valid_tokens_are_reported_as_expired() {
        let secret = "a-strong-shared-secret-with-more-than-32-bytes!";
        let token_service = Arc::new(TokenService::new(secret, "flare-im-core", 3600));
        let composite = CompositeTokenValidator::new(token_service.clone()).with_trusted_issuer(
            "social-secret-with-more-than-32-bytes-long!",
            "flare-social",
        );
        let local = CoreJwtTokenValidator::new(composite.clone());

        assert!(composite.is_expired(&expired_core_token(secret)));
        let live = token_service
            .generate_token("user-a", None, Some("tenant-a"))
            .unwrap();
        assert!(!composite.is_expired(&live));
        for token in [
            "garbage".to_string(),
            expired_core_token("another-secret-with-more-than-32-bytes-long!!"),
        ] {
            assert!(!composite.is_expired(&token));
            let err = local.validate(req(&token)).await.unwrap_err();
            assert!(
                matches!(&err, AuthError::InvalidToken(_)) && !err.is_expired_token(),
                "{err}"
            );
        }
    }

    #[test]
    fn expired_token_marker_is_a_prefix_token() {
        assert!(AuthError::expired_token("ExpiredSignature").is_expired_token());
        assert!(AuthError::InvalidToken(TOKEN_EXPIRED_REASON.into()).is_expired_token());
        assert!(!AuthError::InvalidToken("TOKEN_EXPIREDX".into()).is_expired_token());
        assert!(
            !AuthError::InvalidToken("invalid token: ExpiredSignature".into()).is_expired_token()
        );
        assert!(!AuthError::ProviderUnavailable("TOKEN_EXPIRED".into()).is_expired_token());
    }

    /// hook 认了令牌却没给 user_id：是确定的拒绝（Forbidden），不是可重试的不可用。
    #[test]
    fn hook_active_principal_without_user_id_is_forbidden() {
        for user_id in [None, Some("   ".to_string())] {
            let response = HookValidationResponse {
                active: true,
                reason: None,
                user_id,
                tenant_id: Some("tenant-a".to_string()),
                device_id: None,
                app_id: None,
                expires_at: None,
                scopes: Vec::new(),
                metadata: HashMap::new(),
            };
            let err = response.into_principal().unwrap_err();
            assert!(matches!(err, AuthError::Forbidden(_)), "{err}");
        }
    }

    #[tokio::test]
    async fn chained_forbidden_short_circuits() {
        let forbid = Arc::new(StubValidator {
            accepts: None,
            result: std::sync::Arc::new(std::sync::Mutex::new(Some(AuthError::Forbidden(
                "nope".into(),
            )))),
        });
        let never = stub("never");
        let chain = ChainedTokenValidator::new(vec![forbid, never]);
        assert!(matches!(
            chain.validate(req("x")).await.unwrap_err(),
            AuthError::Forbidden(_)
        ));
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

    #[tokio::test]
    async fn core_jwt_validator_rejects_refresh_and_control_tokens() {
        let token_service = Arc::new(TokenService::new("secret", "flare-im-core", 3600));
        let refresh = token_service
            .generate_refresh_token("user-a", Some("device-a"), Some("tenant-a"))
            .expect("refresh token");
        let control = token_service
            .generate_token_with_use(
                "operator-a",
                None,
                Some("tenant-a"),
                Duration::from_secs(60),
                Some("control"),
            )
            .expect("control token");
        let validator = CoreJwtTokenValidator::new(CompositeTokenValidator::new(token_service));
        let chain = ChainedTokenValidator::new(vec![Arc::new(validator.clone()), stub(&refresh)]);
        assert!(matches!(
            chain
                .validate(TokenValidationRequest {
                    token: refresh.clone(),
                    trace_id: None,
                    request_id: None,
                    path: None,
                    method: None,
                })
                .await,
            Err(AuthError::Forbidden(_))
        ));
        for token in [refresh, control] {
            assert!(matches!(
                validator
                    .validate(TokenValidationRequest {
                        token,
                        trace_id: None,
                        request_id: None,
                        path: None,
                        method: None,
                    })
                    .await,
                Err(AuthError::Forbidden(_))
            ));
        }
    }
}
