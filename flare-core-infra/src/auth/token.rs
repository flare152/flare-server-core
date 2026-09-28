use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use hmac::{Hmac, Mac};
use jsonwebtoken::crypto::{CryptoProvider, JwkUtils, JwtSigner, JwtVerifier};
use jsonwebtoken::errors::{ErrorKind, Result as JwtResult, new_error};
use jsonwebtoken::signature::{Signer, Verifier};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Sha384, Sha512};
use uuid::Uuid;

use super::store::TokenStore;

type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;
type HmacSha512 = Hmac<Sha512>;

macro_rules! define_hmac_signer {
    ($name:ident, $alg:expr, $hmac_type:ty) => {
        struct $name($hmac_type);

        impl $name {
            fn new(encoding_key: &EncodingKey) -> JwtResult<Self> {
                let inner = <$hmac_type>::new_from_slice(encoding_key.try_get_hmac_secret()?)
                    .map_err(|_| new_error(ErrorKind::InvalidKeyFormat))?;
                Ok(Self(inner))
            }
        }

        impl Signer<Vec<u8>> for $name {
            fn try_sign(
                &self,
                msg: &[u8],
            ) -> std::result::Result<Vec<u8>, jsonwebtoken::signature::Error> {
                let mut signer = self.0.clone();
                signer.update(msg);
                Ok(signer.finalize().into_bytes().to_vec())
            }
        }

        impl JwtSigner for $name {
            fn algorithm(&self) -> Algorithm {
                $alg
            }
        }
    };
}

macro_rules! define_hmac_verifier {
    ($name:ident, $alg:expr, $hmac_type:ty) => {
        struct $name($hmac_type);

        impl $name {
            fn new(decoding_key: &DecodingKey) -> JwtResult<Self> {
                let inner = <$hmac_type>::new_from_slice(decoding_key.try_get_hmac_secret()?)
                    .map_err(|_| new_error(ErrorKind::InvalidKeyFormat))?;
                Ok(Self(inner))
            }
        }

        impl Verifier<Vec<u8>> for $name {
            fn verify(
                &self,
                msg: &[u8],
                signature: &Vec<u8>,
            ) -> std::result::Result<(), jsonwebtoken::signature::Error> {
                let mut verifier = self.0.clone();
                verifier.update(msg);
                verifier
                    .verify_slice(signature)
                    .map_err(jsonwebtoken::signature::Error::from_source)
            }
        }

        impl JwtVerifier for $name {
            fn algorithm(&self) -> Algorithm {
                $alg
            }
        }
    };
}

define_hmac_signer!(Hs256Signer, Algorithm::HS256, HmacSha256);
define_hmac_signer!(Hs384Signer, Algorithm::HS384, HmacSha384);
define_hmac_signer!(Hs512Signer, Algorithm::HS512, HmacSha512);
define_hmac_verifier!(Hs256Verifier, Algorithm::HS256, HmacSha256);
define_hmac_verifier!(Hs384Verifier, Algorithm::HS384, HmacSha384);
define_hmac_verifier!(Hs512Verifier, Algorithm::HS512, HmacSha512);

struct Ed25519Signer(SigningKey);

impl Signer<Vec<u8>> for Ed25519Signer {
    fn try_sign(&self, msg: &[u8]) -> std::result::Result<Vec<u8>, jsonwebtoken::signature::Error> {
        Ok(self.0.sign(msg).to_bytes().to_vec())
    }
}

impl JwtSigner for Ed25519Signer {
    fn algorithm(&self) -> Algorithm {
        Algorithm::EdDSA
    }
}

struct Ed25519Verifier(VerifyingKey);

impl Verifier<Vec<u8>> for Ed25519Verifier {
    fn verify(
        &self,
        msg: &[u8],
        signature: &Vec<u8>,
    ) -> std::result::Result<(), jsonwebtoken::signature::Error> {
        self.0
            .verify_strict(msg, &Signature::from_slice(signature)?)
    }
}

impl JwtVerifier for Ed25519Verifier {
    fn algorithm(&self) -> Algorithm {
        Algorithm::EdDSA
    }
}

static FLARE_JWT_PROVIDER: CryptoProvider = CryptoProvider {
    signer_factory: hmac_signer_factory,
    verifier_factory: hmac_verifier_factory,
    jwk_utils: JwkUtils::new_unimplemented(),
};

fn hmac_signer_factory(algorithm: &Algorithm, key: &EncodingKey) -> JwtResult<Box<dyn JwtSigner>> {
    match algorithm {
        Algorithm::HS256 => Ok(Box::new(Hs256Signer::new(key)?)),
        Algorithm::HS384 => Ok(Box::new(Hs384Signer::new(key)?)),
        Algorithm::HS512 => Ok(Box::new(Hs512Signer::new(key)?)),
        Algorithm::EdDSA => Ok(Box::new(Ed25519Signer(
            SigningKey::from_pkcs8_der(key.inner())
                .map_err(|_| new_error(ErrorKind::InvalidEddsaKey))?,
        ))),
        _ => Err(new_error(ErrorKind::InvalidAlgorithm)),
    }
}

fn hmac_verifier_factory(
    algorithm: &Algorithm,
    key: &DecodingKey,
) -> JwtResult<Box<dyn JwtVerifier>> {
    match algorithm {
        Algorithm::HS256 => Ok(Box::new(Hs256Verifier::new(key)?)),
        Algorithm::HS384 => Ok(Box::new(Hs384Verifier::new(key)?)),
        Algorithm::HS512 => Ok(Box::new(Hs512Verifier::new(key)?)),
        Algorithm::EdDSA => {
            // Exact-length conversion avoids panicking or accepting truncated keys.
            let bytes: &[u8; 32] = key
                .as_bytes()
                .try_into()
                .map_err(|_| new_error(ErrorKind::InvalidEddsaKey))?;
            Ok(Box::new(Ed25519Verifier(
                VerifyingKey::from_bytes(bytes)
                    .map_err(|_| new_error(ErrorKind::InvalidEddsaKey))?,
            )))
        }
        _ => Err(new_error(ErrorKind::InvalidAlgorithm)),
    }
}

fn install_hmac_jwt_provider() {
    let _ = FLARE_JWT_PROVIDER.install_default();
}

/// Decode PEM directly or a base64-encoded PEM/DER document. Never include key material in errors.
fn key_document(material: &str) -> Result<Vec<u8>> {
    if material.trim_start().starts_with("-----BEGIN ") {
        Ok(material.as_bytes().to_vec())
    } else {
        STANDARD
            .decode(material.trim())
            .map_err(|_| anyhow!("invalid base64 key document"))
    }
}

#[derive(Serialize, Deserialize)]
struct AudienceClaims {
    #[serde(flatten)]
    claims: TokenClaims,
    aud: String,
}

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
    /// 令牌用途：`Some("refresh")` 为长效刷新令牌，缺省/`access` 为接入令牌。
    /// 老令牌没有这个字段，反序列化为 `None`，按接入令牌处理（向后兼容）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_use: Option<String>,
}

/// refresh 令牌的 `token_use` 取值。
pub const TOKEN_USE_REFRESH: &str = "refresh";
/// 控制面 → 数据面服务间令牌的 `token_use` 取值。
pub const TOKEN_USE_CONTROL: &str = "control";
/// 核 admin-gateway 管理令牌的 `token_use` 取值:校验后派生 `admin:*` scope。
pub const TOKEN_USE_ADMIN: &str = "admin";

/// Token service using an explicit HS256 or Ed25519 key. Public-key instances cannot sign.
pub struct TokenService {
    secret: String,
    issuer: String,
    ttl: Duration,
    /// 刷新令牌有效期（长效）。默认 30 天，可经 `with_refresh_ttl` 配置。
    refresh_ttl: Duration,
    encoding_key: Option<EncodingKey>,
    decoding_key: DecodingKey,
    algorithm: Algorithm,
    store: Option<Arc<dyn TokenStore>>,
}

/// 刷新令牌默认有效期：30 天。
pub const DEFAULT_REFRESH_TTL_SECS: u64 = 30 * 24 * 60 * 60;

impl TokenService {
    /// Creates a new token service
    pub fn new(secret: impl Into<String>, issuer: impl Into<String>, ttl_seconds: u64) -> Self {
        install_hmac_jwt_provider();

        let secret = secret.into();
        let issuer = issuer.into();
        let encoding_key = EncodingKey::from_secret(secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(secret.as_bytes());
        Self {
            secret,
            issuer,
            ttl: Duration::from_secs(ttl_seconds.max(60)),
            refresh_ttl: Duration::from_secs(DEFAULT_REFRESH_TTL_SECS),
            encoding_key: Some(encoding_key),
            decoding_key,
            algorithm: Algorithm::HS256,
            store: None,
        }
    }

    /// Console/internal issuer. Accept PKCS#8 PEM, base64 PEM or base64 DER.
    pub fn from_ed25519_private_key(
        material: &str,
        issuer: impl Into<String>,
        ttl_seconds: u64,
    ) -> Result<Self> {
        install_hmac_jwt_provider();
        let document = key_document(material)?;
        let key = if document.starts_with(b"-----BEGIN ") {
            SigningKey::from_pkcs8_pem(
                std::str::from_utf8(&document).map_err(|_| anyhow!("invalid key encoding"))?,
            )
        } else {
            SigningKey::from_pkcs8_der(&document)
        }
        .map_err(|_| anyhow!("invalid Ed25519 private key"))?;
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        let der = key
            .to_pkcs8_der()
            .map_err(|_| anyhow!("invalid Ed25519 private key"))?;
        Ok(Self {
            secret: String::new(),
            issuer: issuer.into(),
            ttl: Duration::from_secs(ttl_seconds.max(1)),
            refresh_ttl: Duration::from_secs(DEFAULT_REFRESH_TTL_SECS),
            encoding_key: Some(EncodingKey::from_ed_der(der.as_bytes())),
            decoding_key: DecodingKey::from_ed_der(key.verifying_key().as_bytes()),
            algorithm: Algorithm::EdDSA,
            store: None,
        })
    }

    /// Data-plane verifier. A public key must never grant signing capability.
    pub fn from_ed25519_public_key(material: &str, issuer: impl Into<String>) -> Result<Self> {
        install_hmac_jwt_provider();
        let document = key_document(material)?;
        let key = if document.starts_with(b"-----BEGIN ") {
            VerifyingKey::from_public_key_pem(
                std::str::from_utf8(&document).map_err(|_| anyhow!("invalid key encoding"))?,
            )
        } else {
            VerifyingKey::from_public_key_der(&document)
        }
        .map_err(|_| anyhow!("invalid Ed25519 public key"))?;
        Ok(Self {
            secret: String::new(),
            issuer: issuer.into(),
            ttl: Duration::from_secs(60),
            refresh_ttl: Duration::from_secs(DEFAULT_REFRESH_TTL_SECS),
            encoding_key: None,
            decoding_key: DecodingKey::from_ed_der(key.as_bytes()),
            algorithm: Algorithm::EdDSA,
            store: None,
        })
    }

    /// Ed25519 tokens always carry an explicit audience and use.
    pub fn generate_with_ed25519(
        &self,
        subject: &str,
        tenant_id: Option<&str>,
        token_use: &str,
        aud: &str,
    ) -> Result<String> {
        if self.algorithm != Algorithm::EdDSA
            || subject.is_empty()
            || token_use.is_empty()
            || aud.is_empty()
        {
            return Err(anyhow!(
                "Ed25519 issuer, subject, token use and audience are required"
            ));
        }
        let key = self
            .encoding_key
            .as_ref()
            .ok_or_else(|| anyhow!("public-key verifier cannot sign tokens"))?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        let claims = AudienceClaims {
            aud: aud.to_owned(),
            claims: TokenClaims {
                sub: subject.to_owned(),
                iss: self.issuer.clone(),
                iat: now,
                exp: now + self.ttl.as_secs() as usize,
                jti: Uuid::new_v4().to_string(),
                device_id: None,
                tenant_id: tenant_id.map(str::to_owned),
                token_use: Some(token_use.to_owned()),
            },
        };
        let token = encode(&Header::new(Algorithm::EdDSA), &claims, key)?;
        if let Some(store) = self.store.as_ref() {
            store.register(subject, &claims.claims.jti, self.ttl)?;
        }
        Ok(token)
    }

    pub fn validate_ed25519_with_use(
        &self,
        token: &str,
        expected_use: &str,
        aud: &str,
    ) -> Result<TokenClaims> {
        if self.algorithm != Algorithm::EdDSA || expected_use.is_empty() || aud.is_empty() {
            return Err(anyhow!(
                "Ed25519 verifier, token use and audience are required"
            ));
        }
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[aud]);
        validation.set_required_spec_claims(&["exp", "iat", "iss", "sub", "aud"]);
        validation.leeway = 0;
        let claims = decode::<AudienceClaims>(token, &self.decoding_key, &validation)?
            .claims
            .claims;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as usize;
        if claims.token_use.as_deref() != Some(expected_use)
            || claims.sub.is_empty()
            || claims.jti.is_empty()
            || claims.iat > now
            || claims.exp <= now
            || claims.exp <= claims.iat
        {
            return Err(anyhow!(
                "invalid token purpose, identity or validity interval"
            ));
        }
        if let Some(store) = self.store.as_ref()
            && store.is_revoked(&claims.jti)?
        {
            return Err(anyhow!("token revoked"));
        }
        Ok(claims)
    }

    fn hmac_encoding_key(&self) -> Result<&EncodingKey> {
        if self.algorithm != Algorithm::HS256 {
            return Err(anyhow!(
                "Ed25519 requires an explicit audience and token use"
            ));
        }
        self.encoding_key
            .as_ref()
            .ok_or_else(|| anyhow!("signing key missing"))
    }

    /// Attach an external token store (e.g. Redis)
    pub fn with_store(mut self, store: Arc<dyn TokenStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// 配置刷新令牌有效期（秒）。不小于接入令牌 TTL、不小于 60s。
    pub fn with_refresh_ttl(mut self, refresh_ttl_secs: u64) -> Self {
        let secs = refresh_ttl_secs.max(self.ttl.as_secs()).max(60);
        self.refresh_ttl = Duration::from_secs(secs);
        self
    }

    /// 刷新令牌有效期。
    pub fn refresh_ttl(&self) -> Duration {
        self.refresh_ttl
    }

    /// Issues a token for the given user/device/tenant
    pub fn generate_token(
        &self,
        user_id: &str,
        device_id: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<String> {
        self.generate_token_with_ttl(user_id, device_id, tenant_id, self.ttl)
    }

    /// Issues a token with an explicit TTL (clamped to at least 60s and at most the service TTL).
    ///
    /// 签发接口允许调用方请求更短的有效期，但绝不能比网关配置的 TTL 更长：
    /// 否则一枚 refresh 就能把 token 拉成永不过期。
    pub fn generate_token_with_ttl(
        &self,
        user_id: &str,
        device_id: Option<&str>,
        tenant_id: Option<&str>,
        ttl: Duration,
    ) -> Result<String> {
        self.generate_token_with_use(user_id, device_id, tenant_id, ttl, None)
    }

    /// 签发一枚带显式用途(`token_use`)的短期令牌。控制面 → 数据面的服务间令牌走这里
    /// (`token_use = "control"`),接收方用 [`validate_token_with_use`](Self::validate_token_with_use)
    /// 校验用途,避免一枚普通接入令牌被拿去打管理接口。
    pub fn generate_token_with_use(
        &self,
        user_id: &str,
        device_id: Option<&str>,
        tenant_id: Option<&str>,
        ttl: Duration,
        token_use: Option<&str>,
    ) -> Result<String> {
        let ttl = ttl.clamp(Duration::from_secs(60), self.ttl);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| anyhow!("system time error: {err}"))?;
        let iat = now.as_secs() as usize;
        let exp = (now + ttl).as_secs() as usize;

        let claims = TokenClaims {
            sub: user_id.to_string(),
            iss: self.issuer.clone(),
            exp,
            iat,
            jti: Uuid::new_v4().to_string(),
            device_id: device_id.map(|v| v.to_string()),
            tenant_id: tenant_id.map(|v| v.to_string()),
            token_use: token_use.map(|v| v.to_string()),
        };

        let token = encode(&Header::default(), &claims, self.hmac_encoding_key()?)
            .map_err(|err| anyhow!("failed to encode token: {err}"))?;

        if let Some(store) = self.store.as_ref() {
            store.register(&claims.sub, &claims.jti, ttl)?;
        }

        Ok(token)
    }

    /// 签发一枚长效刷新令牌（`token_use = "refresh"`，TTL = 配置的刷新有效期）。
    /// 刷新令牌只用于换新接入令牌，绝不能拿去连长连接（网关校验会拒绝非 access 用途）。
    pub fn generate_refresh_token(
        &self,
        user_id: &str,
        device_id: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| anyhow!("system time error: {err}"))?;
        let iat = now.as_secs() as usize;
        let exp = (now + self.refresh_ttl).as_secs() as usize;
        let claims = TokenClaims {
            sub: user_id.to_string(),
            iss: self.issuer.clone(),
            exp,
            iat,
            jti: Uuid::new_v4().to_string(),
            device_id: device_id.map(|v| v.to_string()),
            tenant_id: tenant_id.map(|v| v.to_string()),
            token_use: Some(TOKEN_USE_REFRESH.to_string()),
        };
        let token = encode(&Header::default(), &claims, self.hmac_encoding_key()?)
            .map_err(|err| anyhow!("failed to encode refresh token: {err}"))?;
        if let Some(store) = self.store.as_ref() {
            store.register(&claims.sub, &claims.jti, self.refresh_ttl)?;
        }
        Ok(token)
    }

    /// 校验刷新令牌：签名 + 发行方 + **必须未过期**（刷新令牌不给宽限）+ 用途为 refresh + 未撤销。
    pub fn decode_refresh_claims(&self, token: &str) -> Result<TokenClaims> {
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        let claims = decode::<TokenClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|err| anyhow!("invalid refresh token: {err}"))?;
        if claims.token_use.as_deref() != Some(TOKEN_USE_REFRESH) {
            return Err(anyhow!("not a refresh token"));
        }
        if let Some(store) = self.store.as_ref()
            && store.is_revoked(&claims.jti)?
        {
            return Err(anyhow!("refresh token revoked"));
        }
        Ok(claims)
    }

    /// Decodes and verifies signature + issuer, but tolerates an expired `exp`.
    ///
    /// 刷新接口用：token 已过期但仍在宽限期内也允许换新，调用方自己检查 `exp` 与宽限。
    /// 撤销检查照做——被撤销的 token 不能靠"过期"绕过。
    pub fn decode_claims_allow_expired(&self, token: &str) -> Result<TokenClaims> {
        let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.validate_exp = false;
        let claims = decode::<TokenClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|err| anyhow!("invalid token: {err}"))?;
        if let Some(store) = self.store.as_ref()
            && store.is_revoked(&claims.jti)?
        {
            return Err(anyhow!("token revoked"));
        }
        Ok(claims)
    }

    /// Revokes one token (no-op without a store).
    pub fn revoke(&self, claims: &TokenClaims, ttl: Duration) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store.revoke(&claims.sub, &claims.jti, ttl)?;
        }
        Ok(())
    }

    /// Whether a revocation store is attached (refresh can rotate only with one).
    pub fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// 撤销某用户的全部令牌（无 store 时为空操作）。
    pub fn revoke_user(&self, user_id: &str, ttl: Duration) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store.revoke_user(user_id, ttl)?;
        }
        Ok(())
    }

    /// 发布「踢用户」信号，通知长连接网关关闭该用户活连接（撤销即断）。无 store 时空操作。
    pub fn publish_kick(&self, user_id: &str) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store.publish_kick(user_id)?;
        }
        Ok(())
    }

    /// 校验令牌且要求 `token_use` 恰为 `expected`(缺省用途视为不匹配)。
    pub fn validate_token_with_use(&self, token: &str, expected: &str) -> Result<TokenClaims> {
        let claims = self.validate_token(token)?;
        if claims.token_use.as_deref() != Some(expected) {
            return Err(anyhow!("token_use mismatch: expected {expected}"));
        }
        Ok(claims)
    }

    /// Validates the token and returns the decoded claims
    pub fn validate_token(&self, token: &str) -> Result<TokenClaims> {
        let claims = self.decode_claims(token)?;

        if let Some(store) = self.store.as_ref()
            && store.is_revoked(&claims.jti)?
        {
            return Err(anyhow!("token revoked"));
        }

        Ok(claims)
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

#[cfg(test)]
mod tests {
    use super::TokenService;
    use super::*;
    use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};

    fn ed_services() -> (TokenService, TokenService) {
        let key = SigningKey::from_bytes(&[42; 32]);
        let private = key.to_pkcs8_pem(Default::default()).unwrap();
        let public = key
            .verifying_key()
            .to_public_key_pem(Default::default())
            .unwrap();
        (
            TokenService::from_ed25519_private_key(&private, "test-issuer", 60).unwrap(),
            TokenService::from_ed25519_public_key(&public, "test-issuer").unwrap(),
        )
    }

    fn custom_ed_token(
        service: &TokenService,
        update: impl FnOnce(&mut serde_json::Value),
    ) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut claims = serde_json::json!({"sub":"employee-7", "iss":"test-issuer", "iat":now, "exp":now+60,
            "jti":"test-id", "tenant_id":"env-a", "token_use":"control", "aud":"flare-internal"});
        update(&mut claims);
        encode(
            &Header::new(Algorithm::EdDSA),
            &claims,
            service.encoding_key.as_ref().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn ed25519_roundtrip_preserves_actor_and_tenant() {
        let (issuer, verifier) = ed_services();
        let token = issuer
            .generate_with_ed25519("employee-7", Some("env-a"), "control", "flare-internal")
            .unwrap();
        let claims = verifier
            .validate_ed25519_with_use(&token, "control", "flare-internal")
            .unwrap();
        assert_eq!(claims.sub, "employee-7");
        assert_eq!(claims.tenant_id.as_deref(), Some("env-a"));
        assert_eq!(claims.exp - claims.iat, 60);
        assert_eq!(
            jsonwebtoken::decode_header(&token).unwrap().alg,
            Algorithm::EdDSA
        );
    }

    #[test]
    fn ed25519_rejects_wrong_audience_use_issuer_and_key() {
        let (issuer, verifier) = ed_services();
        let token = issuer
            .generate_with_ed25519("employee", None, "control", "flare-internal")
            .unwrap();
        assert!(
            verifier
                .validate_ed25519_with_use(&token, "control", "other-aud")
                .is_err()
        );
        assert!(
            verifier
                .validate_ed25519_with_use(&token, "access", "flare-internal")
                .is_err()
        );
        let wrong_issuer = custom_ed_token(&issuer, |c| c["iss"] = "attacker".into());
        assert!(
            verifier
                .validate_ed25519_with_use(&wrong_issuer, "control", "flare-internal")
                .is_err()
        );
        let other_key = SigningKey::from_bytes(&[43; 32])
            .verifying_key()
            .to_public_key_pem(Default::default())
            .unwrap();
        let other = TokenService::from_ed25519_public_key(&other_key, "test-issuer").unwrap();
        assert!(
            other
                .validate_ed25519_with_use(&token, "control", "flare-internal")
                .is_err()
        );
    }

    #[test]
    fn ed25519_rejects_expired_future_and_missing_claims_without_leeway() {
        let (issuer, verifier) = ed_services();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for field in ["exp", "iat", "iss", "sub", "aud", "token_use", "jti"] {
            let token = custom_ed_token(&issuer, |c| {
                c.as_object_mut().unwrap().remove(field);
            });
            assert!(
                verifier
                    .validate_ed25519_with_use(&token, "control", "flare-internal")
                    .is_err(),
                "accepted missing {field}"
            );
        }
        for exp in [now - 1, now] {
            let token = custom_ed_token(&issuer, |c| {
                c["iat"] = (now - 60).into();
                c["exp"] = exp.into();
            });
            assert!(
                verifier
                    .validate_ed25519_with_use(&token, "control", "flare-internal")
                    .is_err()
            );
        }
        let token = custom_ed_token(&issuer, |c| c["iat"] = (now + 30).into());
        assert!(
            verifier
                .validate_ed25519_with_use(&token, "control", "flare-internal")
                .is_err()
        );
    }

    #[test]
    fn ed25519_rejects_payload_tampering_and_hmac_confusion() {
        let (issuer, verifier) = ed_services();
        let token = issuer
            .generate_with_ed25519("employee", None, "control", "flare-internal")
            .unwrap();
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
        parts[1] =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"attacker"}"#);
        assert!(
            verifier
                .validate_ed25519_with_use(&parts.join("."), "control", "flare-internal")
                .is_err()
        );
        let hmac = TokenService::new("test-secret", "test-issuer", 60)
            .generate_token_with_use(
                "attacker",
                None,
                None,
                Duration::from_secs(60),
                Some("control"),
            )
            .unwrap();
        assert!(
            verifier
                .validate_ed25519_with_use(&hmac, "control", "flare-internal")
                .is_err()
        );
        assert!(
            verifier
                .generate_with_ed25519("employee", None, "control", "flare-internal")
                .is_err()
        );
        assert!(issuer.generate_token("employee", None, None).is_err());
        assert!(issuer.validate_token(&hmac).is_err());
    }

    #[test]
    fn ed25519_accepts_base64_key_documents_and_rejects_malformed_keys() {
        let key = SigningKey::from_bytes(&[42; 32]);
        let private = STANDARD.encode(key.to_pkcs8_der().unwrap().as_bytes());
        let public = STANDARD.encode(
            key.verifying_key()
                .to_public_key_pem(Default::default())
                .unwrap(),
        );
        let issuer = TokenService::from_ed25519_private_key(&private, "test-issuer", 60).unwrap();
        let verifier = TokenService::from_ed25519_public_key(&public, "test-issuer").unwrap();
        let token = issuer
            .generate_with_ed25519("employee", None, "control", "flare-internal")
            .unwrap();
        assert!(
            verifier
                .validate_ed25519_with_use(&token, "control", "flare-internal")
                .is_ok()
        );
        for material in ["", "not-base64", "c2hvcnQ="] {
            assert!(TokenService::from_ed25519_private_key(material, "test", 60).is_err());
            assert!(TokenService::from_ed25519_public_key(material, "test").is_err());
        }
        assert!(
            hmac_verifier_factory(&Algorithm::EdDSA, &DecodingKey::from_ed_der(&[0; 4])).is_err()
        );
    }

    #[test]
    fn hs256_token_roundtrip_uses_hmac_provider() {
        let service = TokenService::new("insecure-secret", "flare-im-core", 3600);
        let token = service
            .generate_token("user-1", Some("device-1"), Some("tenant-1"))
            .expect("token should be generated with HMAC provider");

        let claims = service
            .validate_token(&token)
            .expect("token should be validated with HMAC provider");

        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.iss, "flare-im-core");
        assert_eq!(claims.device_id.as_deref(), Some("device-1"));
        assert_eq!(claims.tenant_id.as_deref(), Some("tenant-1"));
    }
}
