use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
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

static HMAC_ONLY_JWT_PROVIDER: CryptoProvider = CryptoProvider {
    signer_factory: hmac_signer_factory,
    verifier_factory: hmac_verifier_factory,
    jwk_utils: JwkUtils::new_unimplemented(),
};

fn hmac_signer_factory(algorithm: &Algorithm, key: &EncodingKey) -> JwtResult<Box<dyn JwtSigner>> {
    match algorithm {
        Algorithm::HS256 => Ok(Box::new(Hs256Signer::new(key)?)),
        Algorithm::HS384 => Ok(Box::new(Hs384Signer::new(key)?)),
        Algorithm::HS512 => Ok(Box::new(Hs512Signer::new(key)?)),
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
        _ => Err(new_error(ErrorKind::InvalidAlgorithm)),
    }
}

fn install_hmac_jwt_provider() {
    let _ = HMAC_ONLY_JWT_PROVIDER.install_default();
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
        install_hmac_jwt_provider();

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
        };

        let token = encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|err| anyhow!("failed to encode token: {err}"))?;

        if let Some(store) = self.store.as_ref() {
            store.register(&claims.sub, &claims.jti, ttl)?;
        }

        Ok(token)
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
