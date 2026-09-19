//! 接入 token 的签发与刷新（生命周期实现，网关只做路由）。
//!
//! 客户端从此不再本地签发：core-only 形态由网关调用这里签发/刷新；
//! `http_hook` 形态转发给业务认证系统。见 `flare-im-core/docs/AUTH-TOKEN-ISSUANCE.zh-CN.md`。

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::header::HeaderName;
use serde::{Deserialize, Serialize};

use super::{AuthError, AuthProviderConfig, AuthProviderMode, TokenService};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenIssueRequest {
    pub user_id: String,
    pub tenant_id: Option<String>,
    pub device_id: Option<String>,
    /// 请求的有效期；`None` 或超出服务 TTL 时按服务 TTL。
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuedToken {
    pub token: String,
    pub expires_at: u64,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// 长效刷新令牌：接入令牌过期后凭它换新，无需重新登录（7x24）。
    /// `core_jwt` 形态签发/刷新时下发；`http_hook` 由业务端决定是否下发。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_expires_at: Option<u64>,
}

#[async_trait]
pub trait TokenIssuer: Send + Sync {
    async fn issue(
        &self,
        request: TokenIssueRequest,
    ) -> std::result::Result<IssuedToken, AuthError>;
    /// 用当前 token（可已过期但在宽限期内）换一枚新 token。
    async fn refresh(&self, current_token: &str) -> std::result::Result<IssuedToken, AuthError>;
}

/// `core_jwt`：本地 HS256 签发；有撤销存储时刷新做轮换（旧 jti 作废）。
pub struct CoreJwtTokenIssuer {
    service: Arc<TokenService>,
    refresh_grace: Duration,
}

impl CoreJwtTokenIssuer {
    pub fn new(service: Arc<TokenService>, refresh_grace: Duration) -> Self {
        Self {
            service,
            refresh_grace,
        }
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn issue_sync(
        &self,
        request: &TokenIssueRequest,
    ) -> std::result::Result<IssuedToken, AuthError> {
        let user_id = request.user_id.trim();
        if user_id.is_empty() {
            return Err(AuthError::InvalidToken("user_id is required".into()));
        }
        let ttl = request
            .ttl_secs
            .map(Duration::from_secs)
            .unwrap_or_else(|| self.service.ttl());
        let tenant_id = request
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let device_id = request
            .device_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let token = self
            .service
            .generate_token_with_ttl(user_id, device_id, tenant_id, ttl)
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
        let claims = self
            .service
            .decode_claims_allow_expired(&token)
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
        // 同时下发一枚长效刷新令牌：接入令牌短命也能靠它维持 7x24 会话。
        let refresh_token = self
            .service
            .generate_refresh_token(user_id, device_id, tenant_id)
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
        let refresh_expires_at = Self::now_secs() + self.service.refresh_ttl().as_secs();
        Ok(IssuedToken {
            token,
            expires_at: claims.exp as u64,
            user_id: claims.sub,
            tenant_id: claims.tenant_id,
            device_id: claims.device_id,
            refresh_token: Some(refresh_token),
            refresh_expires_at: Some(refresh_expires_at),
        })
    }
}

#[async_trait]
impl TokenIssuer for CoreJwtTokenIssuer {
    async fn issue(
        &self,
        request: TokenIssueRequest,
    ) -> std::result::Result<IssuedToken, AuthError> {
        self.issue_sync(&request)
    }

    async fn refresh(&self, current_token: &str) -> std::result::Result<IssuedToken, AuthError> {
        let now = Self::now_secs();
        // 优先按**刷新令牌**处理：签名对、未过期、用途 refresh、未撤销就放行。
        // 这是长效凭证，接入令牌早已过期也能换新——7x24 的关键。
        if let Ok(refresh_claims) = self.service.decode_refresh_claims(current_token) {
            let fresh = self.issue_sync(&TokenIssueRequest {
                user_id: refresh_claims.sub.clone(),
                tenant_id: refresh_claims.tenant_id.clone(),
                device_id: refresh_claims.device_id.clone(),
                ttl_secs: None,
            })?;
            // 轮换刷新令牌：旧的作废到它自己到期，杜绝被盗刷新令牌长期可用。
            let refresh_revoke_ttl =
                Duration::from_secs((refresh_claims.exp as u64).saturating_sub(now))
                    .max(Duration::from_secs(1));
            self.service
                .revoke(&refresh_claims, refresh_revoke_ttl)
                .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
            return Ok(fresh);
        }

        // 兼容旧客户端：拿（可能已过期但在宽限内的）接入令牌换新。
        let claims = self
            .service
            .decode_claims_allow_expired(current_token)
            .map_err(|err| AuthError::InvalidToken(err.to_string()))?;
        let exp = claims.exp as u64;
        if exp + self.refresh_grace.as_secs() < now {
            return Err(AuthError::InvalidToken(format!(
                "token expired {}s ago, beyond the refresh grace of {}s",
                now.saturating_sub(exp),
                self.refresh_grace.as_secs()
            )));
        }
        let fresh = self.issue_sync(&TokenIssueRequest {
            user_id: claims.sub.clone(),
            tenant_id: claims.tenant_id.clone(),
            device_id: claims.device_id.clone(),
            ttl_secs: None,
        })?;
        // 轮换：旧 token 作废到它自己的宽限期结束，之后本来也没人认。
        let revoke_ttl = Duration::from_secs(exp.saturating_sub(now)) + self.refresh_grace;
        self.service
            .revoke(&claims, revoke_ttl)
            .map_err(|err| AuthError::ProviderUnavailable(err.to_string()))?;
        Ok(fresh)
    }
}

/// `http_hook`：签发/刷新都转发给业务认证系统，带同一个 hook secret 头。
pub struct HttpHookTokenIssuer {
    client: reqwest::Client,
    endpoint: String,
    secret_header: HeaderName,
    secret: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookIssueRequest<'a> {
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<&'a str>,
}

impl HttpHookTokenIssuer {
    pub fn new(config: &AuthProviderConfig) -> Result<Self> {
        let endpoint = config
            .hook_issue_url
            .clone()
            .filter(|url| !url.trim().is_empty())
            .context("auth hook issue URL is required to issue tokens in http_hook auth mode")?;
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

    async fn call(
        &self,
        body: HookIssueRequest<'_>,
    ) -> std::result::Result<IssuedToken, AuthError> {
        let mut request = self.client.post(&self.endpoint).json(&body);
        if let Some(secret) = self.secret.as_ref() {
            request = request.header(self.secret_header.clone(), secret);
        }
        let response = request.send().await.map_err(|err| {
            AuthError::ProviderUnavailable(format!("auth hook request failed: {err}"))
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(AuthError::InvalidToken(format!(
                "auth hook rejected the request ({status})"
            )));
        }
        if !status.is_success() {
            return Err(AuthError::ProviderUnavailable(format!(
                "auth hook returned {status}"
            )));
        }
        response.json::<IssuedToken>().await.map_err(|err| {
            AuthError::ProviderUnavailable(format!("auth hook returned an invalid body: {err}"))
        })
    }
}

#[async_trait]
impl TokenIssuer for HttpHookTokenIssuer {
    async fn issue(
        &self,
        request: TokenIssueRequest,
    ) -> std::result::Result<IssuedToken, AuthError> {
        self.call(HookIssueRequest {
            action: "issue",
            user_id: Some(request.user_id.as_str()),
            tenant_id: request.tenant_id.as_deref(),
            device_id: request.device_id.as_deref(),
            ttl_secs: request.ttl_secs,
            token: None,
        })
        .await
    }

    async fn refresh(&self, current_token: &str) -> std::result::Result<IssuedToken, AuthError> {
        self.call(HookIssueRequest {
            action: "refresh",
            user_id: None,
            tenant_id: None,
            device_id: None,
            ttl_secs: None,
            token: Some(current_token),
        })
        .await
    }
}

/// 按鉴权模式选签发实现。`http_hook` 未配 issue URL 时返回 `None`（网关据此回 501）。
pub fn build_token_issuer(
    config: &AuthProviderConfig,
    token_service: Arc<TokenService>,
) -> Result<Option<Arc<dyn TokenIssuer>>> {
    match config.mode {
        AuthProviderMode::CoreJwt => Ok(Some(Arc::new(CoreJwtTokenIssuer::new(
            token_service,
            Duration::from_secs(config.refresh_grace_secs),
        )))),
        AuthProviderMode::HttpHook => {
            if config
                .hook_issue_url
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Ok(None);
            }
            Ok(Some(Arc::new(HttpHookTokenIssuer::new(config)?)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn service(ttl: u64) -> Arc<TokenService> {
        Arc::new(TokenService::new(
            "a-strong-shared-secret-with-more-than-32-bytes!",
            "flare-im-core",
            ttl,
        ))
    }

    #[tokio::test]
    async fn issue_carries_identity_and_respects_service_ttl_ceiling() {
        let issuer = CoreJwtTokenIssuer::new(service(3600), Duration::from_secs(60));
        let issued = issuer
            .issue(TokenIssueRequest {
                user_id: " hugo ".into(),
                tenant_id: Some("0".into()),
                device_id: Some("ios-1".into()),
                ttl_secs: Some(86_400),
            })
            .await
            .expect("issue");
        assert_eq!(issued.user_id, "hugo");
        assert_eq!(issued.tenant_id.as_deref(), Some("0"));
        assert_eq!(issued.device_id.as_deref(), Some("ios-1"));
        let now = CoreJwtTokenIssuer::now_secs();
        assert!(
            issued.expires_at <= now + 3600,
            "请求 24h 也不能超过服务 TTL 1h"
        );
        assert!(issued.expires_at >= now + 3600 - 5);
    }

    #[tokio::test]
    async fn issue_rejects_empty_user() {
        let issuer = CoreJwtTokenIssuer::new(service(3600), Duration::from_secs(60));
        let err = issuer
            .issue(TokenIssueRequest::default())
            .await
            .unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken(_)));
    }

    #[tokio::test]
    async fn issue_returns_refresh_token_and_refresh_token_grant_works() {
        let svc = service(3600);
        let issuer = CoreJwtTokenIssuer::new(svc.clone(), Duration::from_secs(60));
        let issued = issuer
            .issue(TokenIssueRequest {
                user_id: "u7".into(),
                tenant_id: Some("0".into()),
                device_id: None,
                ttl_secs: None,
            })
            .await
            .unwrap();
        // 签发同时下发了刷新令牌，且其有效期远长于接入令牌。
        let refresh = issued
            .refresh_token
            .clone()
            .expect("应当下发 refresh token");
        assert!(
            issued.refresh_expires_at.unwrap() > issued.expires_at,
            "refresh 应比 access 长效"
        );
        // 用刷新令牌换新（即使接入令牌已过期也应成立——这里直接用刷新令牌）。
        let renewed = issuer
            .refresh(&refresh)
            .await
            .expect("refresh-token grant 应成功");
        assert_eq!(renewed.user_id, "u7");
        assert!(
            renewed.refresh_token.is_some(),
            "换新应轮换出新的 refresh token"
        );
        // 轮换后旧刷新令牌作废（有 store 时）。
        if svc.has_store() {
            assert!(
                issuer.refresh(&refresh).await.is_err(),
                "旧 refresh token 轮换后应失效"
            );
        }
    }

    #[tokio::test]
    async fn refresh_keeps_identity_and_rejects_wrong_secret() {
        let svc = service(3600);
        let issuer = CoreJwtTokenIssuer::new(svc.clone(), Duration::from_secs(60));
        let first = issuer
            .issue(TokenIssueRequest {
                user_id: "u1".into(),
                tenant_id: Some("t9".into()),
                device_id: None,
                ttl_secs: None,
            })
            .await
            .unwrap();
        let second = issuer.refresh(&first.token).await.expect("refresh");
        assert_eq!(second.user_id, "u1");
        assert_eq!(second.tenant_id.as_deref(), Some("t9"));
        assert_ne!(second.token, first.token);

        let other = CoreJwtTokenIssuer::new(
            Arc::new(TokenService::new(
                "another-secret-that-is-also-long-enough!!",
                "flare-im-core",
                3600,
            )),
            Duration::from_secs(60),
        );
        let err = other.refresh(&first.token).await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidToken(_)), "{err}");
    }

    #[tokio::test]
    async fn refresh_within_grace_ok_beyond_grace_rejected() {
        // ttl 被 clamp 到最少 60s，用 60s 的服务 + 手工构造过期 token 来测宽限。
        let secret = "a-strong-shared-secret-with-more-than-32-bytes!";
        let expired = {
            use jsonwebtoken::{EncodingKey, Header, encode};
            let now = CoreJwtTokenIssuer::now_secs();
            let claims = super::super::TokenClaims {
                sub: "u2".into(),
                iss: "flare-im-core".into(),
                exp: (now - 100) as usize,
                iat: (now - 200) as usize,
                jti: "j-expired".into(),
                device_id: None,
                tenant_id: None,
                token_use: None,
            };
            encode(
                &Header::default(),
                &claims,
                &EncodingKey::from_secret(secret.as_bytes()),
            )
            .unwrap()
        };
        let svc = Arc::new(TokenService::new(secret, "flare-im-core", 3600));
        let generous = CoreJwtTokenIssuer::new(svc.clone(), Duration::from_secs(1000));
        assert!(
            generous.refresh(&expired).await.is_ok(),
            "过期 100s、宽限 1000s：允许换新"
        );
        let strict = CoreJwtTokenIssuer::new(svc, Duration::from_secs(10));
        let err = strict.refresh(&expired).await.unwrap_err();
        assert!(
            matches!(err, AuthError::InvalidToken(_)),
            "过期 100s、宽限 10s：拒绝"
        );
    }

    #[derive(Default)]
    struct MemStore {
        revoked: Mutex<Vec<String>>,
    }
    impl super::super::TokenStore for MemStore {
        fn register(&self, _user_id: &str, _jti: &str, _ttl: Duration) -> Result<()> {
            Ok(())
        }
        fn revoke(&self, _user_id: &str, jti: &str, _ttl: Duration) -> Result<()> {
            self.revoked.lock().unwrap().push(jti.to_string());
            Ok(())
        }
        fn revoke_user(&self, _user_id: &str, _ttl: Duration) -> Result<()> {
            Ok(())
        }
        fn is_revoked(&self, jti: &str) -> Result<bool> {
            Ok(self.revoked.lock().unwrap().iter().any(|j| j == jti))
        }
    }

    #[tokio::test]
    async fn refresh_rotates_when_a_store_is_attached() {
        let store = Arc::new(MemStore::default());
        let svc = Arc::new(
            TokenService::new(
                "a-strong-shared-secret-with-more-than-32-bytes!",
                "flare-im-core",
                3600,
            )
            .with_store(store.clone()),
        );
        let issuer = CoreJwtTokenIssuer::new(svc.clone(), Duration::from_secs(60));
        let first = issuer
            .issue(TokenIssueRequest {
                user_id: "u3".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let _second = issuer.refresh(&first.token).await.unwrap();
        assert_eq!(
            store.revoked.lock().unwrap().len(),
            1,
            "刷新后旧 jti 必须作废"
        );
        let err = issuer.refresh(&first.token).await.unwrap_err();
        assert!(
            matches!(err, AuthError::InvalidToken(_)),
            "作废的 token 不能再换新: {err}"
        );
    }

    #[test]
    fn hook_issuer_requires_issue_url_and_build_returns_none_without_it() {
        let cfg = AuthProviderConfig {
            mode: AuthProviderMode::HttpHook,
            ..Default::default()
        };
        assert!(build_token_issuer(&cfg, service(60)).unwrap().is_none());
        let cfg = AuthProviderConfig {
            mode: AuthProviderMode::HttpHook,
            hook_issue_url: Some("http://127.0.0.1:1/issue".into()),
            ..Default::default()
        };
        assert!(build_token_issuer(&cfg, service(60)).unwrap().is_some());
    }

    #[test]
    fn app_credentials_parse_and_validate() {
        let creds =
            AppCredential::parse_list("app1:0123456789abcdef, app2:fedcba9876543210x").unwrap();
        assert_eq!(creds.len(), 2);
        assert_eq!(creds[0].app_id, "app1");
        assert!(AppCredential::parse_list("nosecret").is_err());
        assert!(
            AppCredential::parse_list("app:short").is_err(),
            "secret 太短必须拒绝"
        );
    }

    use super::super::AppCredential;
}
