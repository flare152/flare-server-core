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

use super::key_set::{
    INTERNAL_CTX_PUBLIC_KEY_ENV, INTERNAL_CTX_PUBLIC_KEYS_ENV, KeySet, LEGACY_KID, valid_kid,
};
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

/// An Ed25519 SPKI public key (PEM, base64 PEM or base64 DER) as a JWT decoding key. The JWT
/// crypto provider is installed on the way, so a verifier built from a [`KeySet`] alone works.
pub(crate) fn ed25519_public_decoding_key(material: &str) -> Result<DecodingKey> {
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
    Ok(DecodingKey::from_ed_der(key.as_bytes()))
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

/// Audience, issuer and purpose are fixed for cell-internal gRPC identity.
pub const INTERNAL_CONTEXT_ISSUER: &str = "flare-internal";
pub const INTERNAL_CONTEXT_AUDIENCE: &str = "flare-internal";
pub const TOKEN_USE_INTERNAL_CONTEXT: &str = "internal_context";
pub const INTERNAL_CONTEXT_TTL_SECS: u64 = 60;

/// A business-neutral actor category carried in the signed internal context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignedActorType {
    User,
    Service,
    TenantAdmin,
    System,
    Guest,
    /// Staff signed by the gateway admin partition after it verified a staff session or a
    /// personal access token. `tenant_id` is the application being administered, or empty for
    /// platform-scoped work; `org_id`, `permission` and `perm_version` describe the grant.
    Staff,
}

/// Identity selected by a trusted caller after it authenticates the original request.
/// The signer sets issuer, audience, purpose, timestamps and nonce itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedContextIdentity {
    pub actor_type: SignedActorType,
    pub actor_id: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    pub credential_id: Option<String>,
    pub request_id: String,
    /// Staff only: the organization the staff member belongs to; None for platform staff.
    pub org_id: Option<String>,
    /// Staff only: the permission code the admin partition matched for this request.
    pub permission: Option<String>,
    /// Staff only: the role bundle version the session was authorized against.
    pub perm_version: Option<u64>,
    /// Trace id of the originating request, for log correlation only; never identity.
    pub trace_id: Option<String>,
}

impl SignedContextIdentity {
    pub fn new(
        actor_type: SignedActorType,
        actor_id: impl Into<String>,
        tenant_id: impl Into<String>,
        request_id: impl Into<String>,
    ) -> Self {
        Self {
            actor_type,
            actor_id: actor_id.into(),
            tenant_id: tenant_id.into(),
            roles: Vec::new(),
            credential_id: None,
            request_id: request_id.into(),
            org_id: None,
            permission: None,
            perm_version: None,
            trace_id: None,
        }
    }

    /// Trace id to propagate (at most 128 bytes); it carries no identity.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Staff grant description. The permission is the route's code as the admin partition
    /// matched it; the receiver still compares it with the operation it serves.
    pub fn with_staff_grant(
        mut self,
        org_id: Option<String>,
        permission: Option<String>,
        perm_version: u64,
    ) -> Self {
        self.org_id = org_id;
        self.permission = permission;
        self.perm_version = Some(perm_version);
        self
    }

    pub fn with_roles(mut self, roles: Vec<String>) -> Self {
        self.roles = roles;
        self
    }

    pub fn with_credential_id(mut self, credential_id: impl Into<String>) -> Self {
        self.credential_id = Some(credential_id.into());
        self
    }

    fn validate(&self) -> Result<()> {
        fn valid(value: &str, max: usize) -> bool {
            !value.is_empty()
                && value.len() <= max
                && value.trim() == value
                && !value.chars().any(char::is_control)
        }

        let staff = self.actor_type == SignedActorType::Staff;
        // Platform-scoped staff work has no application: only Staff may carry an empty tenant.
        let tenant_ok = valid(&self.tenant_id, 256) || (staff && self.tenant_id.is_empty());
        let grant_ok = if staff {
            self.org_id.as_deref().is_none_or(|id| valid(id, 256))
                && self
                    .permission
                    .as_deref()
                    .is_none_or(|code| valid(code, 128))
        } else {
            self.org_id.is_none() && self.permission.is_none() && self.perm_version.is_none()
        };
        if !valid(&self.actor_id, 256)
            || !tenant_ok
            || !grant_ok
            || !valid(&self.request_id, 128)
            || self
                .credential_id
                .as_deref()
                .is_some_and(|id| !valid(id, 256))
            || self.trace_id.as_deref().is_some_and(|id| !valid(id, 128))
            || self.roles.len() > 32
            || self.roles.iter().any(|role| !valid(role, 128))
            || (self.actor_type == SignedActorType::Guest && !self.roles.is_empty())
        {
            return Err(anyhow!("invalid signed context identity"));
        }
        Ok(())
    }
}

/// Exact EdDSA wire claims. `jti` supports downstream idempotency/replay tracking;
/// retry and forwarding deliberately reuse a token within its 60-second validity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedContextClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub token_use: String,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
    pub actor_type: SignedActorType,
    pub actor_id: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    pub request_id: String,
    /// Staff grant (v4 R1-3); absent on every other actor type and on older tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perm_version: Option<u64>,
    /// Trace id of the originating request (v4 R3-1); correlation only, never identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Key id the token was verified with: the JWT `kid` header, [`LEGACY_KID`] when absent.
    /// Filled by the verifier; never part of the signed payload.
    #[serde(skip)]
    pub kid: String,
}

impl SignedContextClaims {
    fn validate(&self, now: u64) -> Result<()> {
        SignedContextIdentity {
            actor_type: self.actor_type,
            actor_id: self.actor_id.clone(),
            tenant_id: self.tenant_id.clone(),
            roles: self.roles.clone(),
            credential_id: self.credential_id.clone(),
            request_id: self.request_id.clone(),
            org_id: self.org_id.clone(),
            permission: self.permission.clone(),
            perm_version: self.perm_version,
            trace_id: self.trace_id.clone(),
        }
        .validate()?;
        if self.sub != self.actor_id
            || self.iss != INTERNAL_CONTEXT_ISSUER
            || self.aud != INTERNAL_CONTEXT_AUDIENCE
            || self.token_use != TOKEN_USE_INTERNAL_CONTEXT
            || Uuid::parse_str(&self.jti).is_err()
            || self.iat > now
            || self.exp <= now
            || self.exp <= self.iat
            || self.exp - self.iat > INTERNAL_CONTEXT_TTL_SECS
        {
            return Err(anyhow!("invalid signed context claims"));
        }
        Ok(())
    }
}

/// Only a process holding the private key can issue cell-internal identity.
///
/// The second field is the key id written into the JWT header (v4 R3-1): the component the
/// private key belongs to, so receivers can resolve the matching public key and pin actor types
/// to components. Without one the token verifies as [`LEGACY_KID`].
#[derive(Clone)]
pub struct SignedContextSigner(Arc<TokenService>, Option<String>);

impl SignedContextSigner {
    pub fn from_private_key(material: &str) -> Result<Self> {
        Ok(Self(
            Arc::new(TokenService::from_ed25519_private_key(
                material,
                INTERNAL_CONTEXT_ISSUER,
                INTERNAL_CONTEXT_TTL_SECS,
            )?),
            None,
        ))
    }

    /// Declares the key id this signer's key is registered under at the receivers.
    pub fn with_kid(mut self, kid: impl Into<String>) -> Result<Self> {
        let kid = kid.into();
        if !valid_kid(&kid) {
            return Err(anyhow!("invalid signed context key id"));
        }
        self.1 = Some(kid);
        Ok(self)
    }

    pub fn kid(&self) -> Option<&str> {
        self.1.as_deref()
    }

    pub fn sign(&self, identity: &SignedContextIdentity) -> Result<String> {
        identity.validate()?;
        let key = self
            .0
            .encoding_key
            .as_ref()
            .ok_or_else(|| anyhow!("public-key verifier cannot sign tokens"))?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let claims = SignedContextClaims {
            sub: identity.actor_id.clone(),
            iss: INTERNAL_CONTEXT_ISSUER.to_owned(),
            aud: INTERNAL_CONTEXT_AUDIENCE.to_owned(),
            token_use: TOKEN_USE_INTERNAL_CONTEXT.to_owned(),
            iat: now,
            exp: now + INTERNAL_CONTEXT_TTL_SECS,
            jti: Uuid::new_v4().to_string(),
            actor_type: identity.actor_type,
            actor_id: identity.actor_id.clone(),
            tenant_id: identity.tenant_id.clone(),
            roles: identity.roles.clone(),
            credential_id: identity.credential_id.clone(),
            request_id: identity.request_id.clone(),
            org_id: identity.org_id.clone(),
            permission: identity.permission.clone(),
            perm_version: identity.perm_version,
            trace_id: identity.trace_id.clone(),
            kid: String::new(),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = self.1.clone();
        Ok(encode(&header, &claims, key)?)
    }
}

/// Public-key-only verifier over a [`KeySet`]. Construction never grants signing capability.
///
/// The JWT `kid` header selects the public key (no kid is [`LEGACY_KID`]); `Staff` tokens may
/// additionally be pinned to the key ids of the components allowed to sign them.
#[derive(Clone)]
pub struct SignedContextVerifier {
    keys: Arc<KeySet>,
    staff_kids: Option<Arc<[String]>>,
}

impl SignedContextVerifier {
    /// The single-key deployment: one Cell public key, registered as [`LEGACY_KID`].
    pub fn from_public_key(material: &str) -> Result<Self> {
        Ok(Self::from_key_set(KeySet::single(material)?))
    }

    pub fn from_key_set(keys: KeySet) -> Self {
        Self {
            keys: Arc::new(keys),
            staff_kids: None,
        }
    }

    /// [`KeySet::from_env`]; an error when neither variable is set.
    pub fn from_env() -> Result<Self> {
        Self::from_env_optional()?.ok_or_else(|| {
            anyhow!("{INTERNAL_CTX_PUBLIC_KEYS_ENV} or {INTERNAL_CTX_PUBLIC_KEY_ENV} is required")
        })
    }

    /// [`KeySet::from_env`]; `None` when neither variable is set.
    pub fn from_env_optional() -> Result<Option<Self>> {
        Ok(KeySet::from_env()?.map(Self::from_key_set))
    }

    /// Accept `Staff` only from these key ids; other actor types are not affected.
    pub fn require_staff_kid<I, K>(mut self, kids: I) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        self.staff_kids = Some(kids.into_iter().map(Into::into).collect());
        self
    }

    pub fn key_set(&self) -> &KeySet {
        &self.keys
    }

    pub fn verify(&self, token: &str) -> Result<SignedContextClaims> {
        let header = jsonwebtoken::decode_header(token)?;
        if header.alg != Algorithm::EdDSA {
            return Err(anyhow!("invalid signed context algorithm"));
        }
        let kid = header.kid.as_deref();
        if let Some(kid) = kid
            && !valid_kid(kid)
        {
            return Err(anyhow!("invalid signed context key id"));
        }
        let key = self
            .keys
            .resolve(kid)
            .ok_or_else(|| anyhow!("unknown signed context key id"))?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[INTERNAL_CONTEXT_ISSUER]);
        validation.set_audience(&[INTERNAL_CONTEXT_AUDIENCE]);
        validation.set_required_spec_claims(&[
            "sub",
            "iss",
            "aud",
            "token_use",
            "iat",
            "exp",
            "jti",
            "actor_type",
            "actor_id",
            "tenant_id",
            "roles",
            "request_id",
        ]);
        validation.leeway = 0;
        let mut claims = decode::<SignedContextClaims>(token, key, &validation)?.claims;
        claims.kid = kid.unwrap_or(LEGACY_KID).to_owned();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        claims.validate(now)?;
        if claims.actor_type == SignedActorType::Staff
            && let Some(allowed) = &self.staff_kids
            && !allowed.iter().any(|accepted| accepted == &claims.kid)
        {
            return Err(anyhow!(
                "staff context signed by a key id this service does not accept"
            ));
        }
        Ok(claims)
    }
}

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
        Ok(Self {
            secret: String::new(),
            issuer: issuer.into(),
            ttl: Duration::from_secs(60),
            refresh_ttl: Duration::from_secs(DEFAULT_REFRESH_TTL_SECS),
            encoding_key: None,
            decoding_key: ed25519_public_decoding_key(material)?,
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
            store.register(
                tenant_id.unwrap_or("0"),
                subject,
                &claims.claims.jti,
                self.ttl,
            )?;
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
            store.register(tenant_id.unwrap_or("0"), &claims.sub, &claims.jti, ttl)?;
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
            store.register(
                tenant_id.unwrap_or("0"),
                &claims.sub,
                &claims.jti,
                self.refresh_ttl,
            )?;
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
            store.revoke(
                claims.tenant_id.as_deref().unwrap_or("0"),
                &claims.sub,
                &claims.jti,
                ttl,
            )?;
        }
        Ok(())
    }

    /// Whether a revocation store is attached (refresh can rotate only with one).
    pub fn has_store(&self) -> bool {
        self.store.is_some()
    }

    /// 撤销某用户的全部令牌（无 store 时为空操作）。
    pub fn revoke_user(&self, tenant_id: &str, user_id: &str, ttl: Duration) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store.revoke_user(tenant_id, user_id, ttl)?;
        }
        Ok(())
    }

    /// 发布「踢用户」信号，通知长连接网关关闭该用户活连接（撤销即断）。无 store 时空操作。
    pub fn publish_kick(&self, signal: &super::store::KickSignal) -> Result<()> {
        if let Some(store) = self.store.as_ref() {
            store.publish_kick(signal)?;
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

    /// Validate a normal end-user access token. Older access tokens have no
    /// `token_use`; refresh/control/admin/internal tokens are never accepted.
    pub fn validate_access_token(&self, token: &str) -> Result<TokenClaims> {
        let claims = self.validate_token(token)?;
        if !matches!(claims.token_use.as_deref(), None | Some("access")) {
            return Err(anyhow!("token is not an access token"));
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

    fn signed_context_services() -> (SignedContextSigner, SignedContextVerifier) {
        let key = SigningKey::from_bytes(&[52; 32]);
        let private = key.to_pkcs8_pem(Default::default()).unwrap();
        let public = key
            .verifying_key()
            .to_public_key_pem(Default::default())
            .unwrap();
        (
            SignedContextSigner::from_private_key(&private).unwrap(),
            SignedContextVerifier::from_public_key(&public).unwrap(),
        )
    }

    fn changed_signed_context(
        signer: &SignedContextSigner,
        update: impl FnOnce(&mut serde_json::Value),
    ) -> String {
        let token = signer
            .sign(&SignedContextIdentity::new(
                SignedActorType::User,
                "user-1",
                "tenant-1",
                "request-1",
            ))
            .unwrap();
        let payload = token.split('.').nth(1).unwrap();
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let mut claims: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        update(&mut claims);
        encode(
            &Header::new(Algorithm::EdDSA),
            &claims,
            signer.0.encoding_key.as_ref().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn signed_context_roundtrip_has_typed_actor_and_bounded_lifetime() {
        let (signer, verifier) = signed_context_services();
        let identity = SignedContextIdentity::new(
            SignedActorType::TenantAdmin,
            "staff-7",
            "tenant-1",
            "request-42",
        )
        .with_roles(vec!["control".to_owned()])
        .with_credential_id("credential-9");
        let token = signer.sign(&identity).unwrap();
        let claims = verifier.verify(&token).unwrap();
        assert_eq!(claims.actor_type, SignedActorType::TenantAdmin);
        assert_eq!(claims.actor_id, "staff-7");
        assert_eq!(claims.tenant_id, "tenant-1");
        assert_eq!(claims.roles, vec!["control"]);
        assert_eq!(claims.credential_id.as_deref(), Some("credential-9"));
        assert_eq!(claims.request_id, "request-42");
        assert_eq!(claims.exp - claims.iat, INTERNAL_CONTEXT_TTL_SECS);
        assert!(Uuid::parse_str(&claims.jti).is_ok());
        assert_eq!(
            jsonwebtoken::decode_header(&token).unwrap().alg,
            Algorithm::EdDSA
        );
    }

    #[test]
    fn staff_identity_carries_its_grant_and_may_be_platform_scoped() {
        let (signer, verifier) = signed_context_services();
        let platform = SignedContextIdentity::new(SignedActorType::Staff, "101", "", "request-1")
            .with_roles(vec!["platform:super_admin".to_owned()])
            .with_staff_grant(None, Some("im:tenants".to_owned()), 9);
        let claims = verifier.verify(&signer.sign(&platform).unwrap()).unwrap();
        assert_eq!(claims.actor_type, SignedActorType::Staff);
        assert_eq!(claims.tenant_id, "");
        assert_eq!(claims.org_id, None);
        assert_eq!(claims.permission.as_deref(), Some("im:tenants"));
        assert_eq!(claims.perm_version, Some(9));
        let organization =
            SignedContextIdentity::new(SignedActorType::Staff, "202", "0", "request-2")
                .with_staff_grant(Some("1000000000000000001".to_owned()), None, 0)
                .with_credential_id("pat:17");
        let claims = verifier
            .verify(&signer.sign(&organization).unwrap())
            .unwrap();
        assert_eq!(claims.org_id.as_deref(), Some("1000000000000000001"));
        assert_eq!(claims.permission, None);
        assert_eq!(claims.credential_id.as_deref(), Some("pat:17"));
    }

    #[test]
    fn only_staff_may_omit_the_tenant_or_carry_a_grant() {
        let (signer, _) = signed_context_services();
        for actor_type in [
            SignedActorType::User,
            SignedActorType::Service,
            SignedActorType::TenantAdmin,
            SignedActorType::System,
            SignedActorType::Guest,
        ] {
            assert!(
                signer
                    .sign(&SignedContextIdentity::new(actor_type, "a", "", "r"))
                    .is_err(),
                "{actor_type:?} with an empty tenant"
            );
            assert!(
                signer
                    .sign(
                        &SignedContextIdentity::new(actor_type, "a", "t", "r")
                            .with_staff_grant(None, None, 1)
                    )
                    .is_err(),
                "{actor_type:?} with a staff grant"
            );
        }
        let staff = SignedContextIdentity::new(SignedActorType::Staff, "a", "t", "r");
        assert!(
            signer
                .sign(&SignedContextIdentity {
                    permission: Some(" padded".to_owned()),
                    ..staff.clone()
                })
                .is_err()
        );
        assert!(
            signer
                .sign(&SignedContextIdentity {
                    org_id: Some(String::new()),
                    ..staff
                })
                .is_err()
        );
    }

    #[test]
    fn tokens_without_grant_fields_still_verify() {
        let (signer, verifier) = signed_context_services();
        let token = changed_signed_context(&signer, |claims| {
            let object = claims.as_object_mut().unwrap();
            object.remove("org_id");
            object.remove("permission");
            object.remove("perm_version");
        });
        let claims = verifier.verify(&token).unwrap();
        assert_eq!(claims.org_id, None);
        assert_eq!(claims.permission, None);
        assert_eq!(claims.perm_version, None);
        let token = changed_signed_context(&signer, |claims| {
            claims["permission"] = serde_json::json!("im:tenants");
        });
        assert!(
            verifier.verify(&token).is_err(),
            "a user token cannot smuggle a staff grant"
        );
    }

    #[test]
    fn signed_context_rejects_wrong_key_purpose_audience_and_tampering() {
        let (signer, verifier) = signed_context_services();
        let identity =
            SignedContextIdentity::new(SignedActorType::User, "user-1", "tenant-1", "request-1");
        let token = signer.sign(&identity).unwrap();
        let other_public = SigningKey::from_bytes(&[53; 32])
            .verifying_key()
            .to_public_key_pem(Default::default())
            .unwrap();
        assert!(
            SignedContextVerifier::from_public_key(&other_public)
                .unwrap()
                .verify(&token)
                .is_err()
        );
        for (field, value) in [
            ("token_use", "control"),
            ("aud", "other-service"),
            ("iss", "other-issuer"),
            ("actor_id", "other-user"),
        ] {
            let token = changed_signed_context(&signer, |claims| claims[field] = value.into());
            assert!(verifier.verify(&token).is_err(), "accepted {field}");
        }
        let mut parts: Vec<_> = token.split('.').map(str::to_owned).collect();
        parts[1] =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"actor_id":"attacker"}"#);
        assert!(verifier.verify(&parts.join(".")).is_err());
        let control = signer
            .0
            .generate_with_ed25519("staff", Some("tenant-1"), "control", "flare-internal")
            .unwrap();
        assert!(verifier.verify(&control).is_err());
        assert!(verifier.key_set().is_single_key());
    }

    fn pem_pair(seed: u8) -> (String, String) {
        let key = SigningKey::from_bytes(&[seed; 32]);
        (
            key.to_pkcs8_pem(Default::default()).unwrap().to_string(),
            key.verifying_key()
                .to_public_key_pem(Default::default())
                .unwrap(),
        )
    }

    fn staff(permission: &str) -> SignedContextIdentity {
        SignedContextIdentity::new(SignedActorType::Staff, "staff-1", "app-1", "request-1")
            .with_staff_grant(None, Some(permission.to_owned()), 3)
    }

    #[test]
    fn kid_header_selects_the_component_key_and_no_kid_is_legacy() {
        let (gateway_private, gateway_public) = pem_pair(61);
        let (legacy_private, legacy_public) = pem_pair(62);
        let mut keys = KeySet::single(&legacy_public).unwrap();
        keys.insert("gateway", &gateway_public).unwrap();
        let verifier = SignedContextVerifier::from_key_set(keys);
        let gateway = SignedContextSigner::from_private_key(&gateway_private)
            .unwrap()
            .with_kid("gateway")
            .unwrap();
        assert_eq!(gateway.kid(), Some("gateway"));
        let legacy = SignedContextSigner::from_private_key(&legacy_private).unwrap();
        assert_eq!(legacy.kid(), None);
        let identity = SignedContextIdentity::new(SignedActorType::User, "u", "t", "r");

        let token = gateway.sign(&identity).unwrap();
        assert_eq!(
            jsonwebtoken::decode_header(&token).unwrap().kid.as_deref(),
            Some("gateway")
        );
        assert_eq!(verifier.verify(&token).unwrap().kid, "gateway");
        let token = legacy.sign(&identity).unwrap();
        assert_eq!(jsonwebtoken::decode_header(&token).unwrap().kid, None);
        assert_eq!(verifier.verify(&token).unwrap().kid, LEGACY_KID);

        // The gateway key without a kid is checked against the legacy key and fails; the
        // legacy key claiming to be the gateway fails; an unregistered kid is unknown.
        let unlabeled_gateway = SignedContextSigner::from_private_key(&gateway_private).unwrap();
        assert!(
            verifier
                .verify(&unlabeled_gateway.sign(&identity).unwrap())
                .is_err()
        );
        let impostor = SignedContextSigner::from_private_key(&legacy_private)
            .unwrap()
            .with_kid("gateway")
            .unwrap();
        assert!(verifier.verify(&impostor.sign(&identity).unwrap()).is_err());
        let hook = SignedContextSigner::from_private_key(&gateway_private)
            .unwrap()
            .with_kid("hook")
            .unwrap();
        assert!(verifier.verify(&hook.sign(&identity).unwrap()).is_err());
        assert!(
            SignedContextSigner::from_private_key(&gateway_private)
                .unwrap()
                .with_kid("bad kid")
                .is_err()
        );
    }

    #[test]
    fn single_key_deployment_verifies_any_kid_until_component_keys_exist() {
        let (private, public) = pem_pair(63);
        let verifier = SignedContextVerifier::from_public_key(&public).unwrap();
        let signer = SignedContextSigner::from_private_key(&private)
            .unwrap()
            .with_kid("gateway")
            .unwrap();
        let identity = SignedContextIdentity::new(SignedActorType::Service, "svc", "t", "r");
        let claims = verifier.verify(&signer.sign(&identity).unwrap()).unwrap();
        assert_eq!(claims.kid, "gateway", "the declared kid is kept");
        let mut keys = KeySet::single(&public).unwrap();
        keys.insert("hook", &pem_pair(64).1).unwrap();
        let pinned = SignedContextVerifier::from_key_set(keys);
        assert!(
            pinned.verify(&signer.sign(&identity).unwrap()).is_err(),
            "a component set no longer falls back to the legacy key"
        );
    }

    #[test]
    fn staff_kid_pinning_rejects_staff_from_other_components_only() {
        let (gateway_private, gateway_public) = pem_pair(65);
        let (hook_private, hook_public) = pem_pair(66);
        let mut keys = KeySet::empty();
        keys.insert("gateway", &gateway_public).unwrap();
        keys.insert("hook", &hook_public).unwrap();
        let verifier = SignedContextVerifier::from_key_set(keys).require_staff_kid(["gateway"]);
        let gateway = SignedContextSigner::from_private_key(&gateway_private)
            .unwrap()
            .with_kid("gateway")
            .unwrap();
        let hook = SignedContextSigner::from_private_key(&hook_private)
            .unwrap()
            .with_kid("hook")
            .unwrap();
        let claims = verifier
            .verify(&gateway.sign(&staff("core:users:kick")).unwrap())
            .unwrap();
        assert_eq!(claims.permission.as_deref(), Some("core:users:kick"));
        assert_eq!(claims.kid, "gateway");
        assert!(
            verifier
                .verify(&hook.sign(&staff("core:users:kick")).unwrap())
                .is_err(),
            "a hook key cannot sign staff"
        );
        let service = SignedContextIdentity::new(SignedActorType::Service, "hook-1", "t", "r");
        assert_eq!(
            verifier.verify(&hook.sign(&service).unwrap()).unwrap().kid,
            "hook"
        );
        let unpinned = SignedContextVerifier::from_key_set(verifier.key_set().clone());
        assert!(
            unpinned
                .verify(&hook.sign(&staff("core:users:kick")).unwrap())
                .is_ok()
        );
    }

    #[test]
    fn trace_id_roundtrips_and_is_bounded() {
        let (signer, verifier) = signed_context_services();
        let identity = SignedContextIdentity::new(SignedActorType::User, "u", "t", "r")
            .with_trace_id("trace-1");
        let claims = verifier.verify(&signer.sign(&identity).unwrap()).unwrap();
        assert_eq!(claims.trace_id.as_deref(), Some("trace-1"));
        let without = verifier
            .verify(
                &signer
                    .sign(&SignedContextIdentity::new(
                        SignedActorType::User,
                        "u",
                        "t",
                        "r",
                    ))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(without.trace_id, None);
        assert!(
            signer
                .sign(&identity.clone().with_trace_id("x".repeat(129)))
                .is_err()
        );
        assert!(signer.sign(&identity.with_trace_id(" padded")).is_err());
        let token = changed_signed_context(&signer, |claims| {
            claims["trace_id"] = serde_json::json!("y".repeat(129));
        });
        assert!(verifier.verify(&token).is_err());
    }

    #[test]
    fn signed_context_rejects_expired_future_overlong_and_missing_claims() {
        let (signer, verifier) = signed_context_services();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        for field in [
            "sub",
            "iss",
            "aud",
            "token_use",
            "iat",
            "exp",
            "jti",
            "actor_type",
            "actor_id",
            "tenant_id",
            "roles",
            "request_id",
        ] {
            let token = changed_signed_context(&signer, |claims| {
                claims.as_object_mut().unwrap().remove(field);
            });
            assert!(verifier.verify(&token).is_err(), "accepted missing {field}");
        }
        for (iat, exp) in [(now - 60, now), (now + 1, now + 60), (now - 61, now + 1)] {
            let token = changed_signed_context(&signer, |claims| {
                claims["iat"] = iat.into();
                claims["exp"] = exp.into();
            });
            assert!(verifier.verify(&token).is_err(), "accepted interval");
        }
    }

    #[test]
    fn signed_context_rejects_invalid_identity_before_issuing() {
        let (signer, _) = signed_context_services();
        let base = SignedContextIdentity::new(
            SignedActorType::Guest,
            "anonymous",
            "tenant-1",
            "request-1",
        );
        assert!(signer.sign(&base).is_ok());
        assert!(
            signer
                .sign(&base.clone().with_roles(vec!["control".into()]))
                .is_err()
        );
        let mut missing_tenant = base.clone();
        missing_tenant.tenant_id.clear();
        assert!(signer.sign(&missing_tenant).is_err());
        let mut bad_request = base;
        bad_request.request_id = "invalid\nheader".to_owned();
        assert!(signer.sign(&bad_request).is_err());
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

    #[test]
    fn normal_access_validation_rejects_refresh_and_control_tokens() {
        let service = TokenService::new("insecure-secret", "flare-im-core", 3600);
        let access = service
            .generate_token("user-1", Some("device-1"), Some("tenant-1"))
            .unwrap();
        let refresh = service
            .generate_refresh_token("user-1", Some("device-1"), Some("tenant-1"))
            .unwrap();
        let control = service
            .generate_token_with_use(
                "operator-1",
                None,
                Some("tenant-1"),
                Duration::from_secs(60),
                Some("control"),
            )
            .unwrap();

        assert!(service.validate_access_token(&access).is_ok());
        assert!(service.validate_access_token(&refresh).is_err());
        assert!(service.validate_access_token(&control).is_err());
    }
}
