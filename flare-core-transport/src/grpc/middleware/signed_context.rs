//! Signed, cell-internal gRPC identity boundary.
//!
//! Only `x-flare-ctx` establishes actor and tenant. Legacy metadata may still be
//! used on services which have not explicitly installed this layer.

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use flare_core_base::context::{ActorContext, ActorType, Context, Ctx};
use flare_core_infra::auth::{SignedActorType, SignedContextClaims, SignedContextVerifier};
use http::{Request as HttpRequest, Response as HttpResponse};
use tonic::Status;
use tonic::body::Body;
use tonic::metadata::MetadataMap;
use tower::{Layer, Service};
use uuid::Uuid;

pub const SIGNED_CONTEXT_HEADER: &str = "x-flare-ctx";

/// Verify one incoming token for a protected RPC on a mixed legacy/new service.
/// The returned Context carries the same private `VerifiedContext` marker as
/// `SignedContextLayer`; caller-provided legacy metadata is never consulted.
pub fn verify_signed_context_metadata(
    metadata: &MetadataMap,
    verifier: &SignedContextVerifier,
) -> Result<Ctx, Status> {
    let mut values = metadata.get_all(SIGNED_CONTEXT_HEADER).iter();
    let token = values
        .next()
        .ok_or_else(|| Status::unauthenticated("signed context required"))?
        .to_str()
        .map_err(|_| Status::unauthenticated("invalid signed context"))?;
    if values.next().is_some() || token.len() > 8192 {
        return Err(Status::unauthenticated("invalid signed context"));
    }
    let claims = verifier
        .verify(token)
        .map_err(|_| Status::unauthenticated("invalid signed context"))?;
    Ok(signed_context(&claims, token))
}

/// Marker created only after verification (or explicit anonymous admission).
/// It cannot be deserialized from headers. `context_utils` uses its presence to
/// prevent legacy metadata from filling blank identity fields.
pub struct VerifiedContext {
    claims: Option<SignedContextClaims>,
    token: Option<String>,
}

impl VerifiedContext {
    fn signed(claims: SignedContextClaims, token: String) -> Self {
        Self {
            claims: Some(claims),
            token: Some(token),
        }
    }

    fn anonymous() -> Self {
        Self {
            claims: None,
            token: None,
        }
    }

    pub fn claims(&self) -> Option<&SignedContextClaims> {
        self.claims.as_ref()
    }

    /// Return the original verified token for forwarding to another service.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}

#[derive(Clone)]
pub struct SignedContextLayer {
    verifier: SignedContextVerifier,
    allow_anonymous: bool,
}

impl SignedContextLayer {
    /// Protected by default: a missing or invalid token is UNAUTHENTICATED.
    pub fn new(verifier: SignedContextVerifier) -> Self {
        Self {
            verifier,
            allow_anonymous: false,
        }
    }

    /// Only for explicitly public services. An invalid supplied token still fails.
    pub fn allow_anonymous(mut self) -> Self {
        self.allow_anonymous = true;
        self
    }

    pub fn layer<S>(&self, service: S) -> <Self as Layer<S>>::Service
    where
        Self: Layer<S>,
    {
        Layer::layer(self, service)
    }
}

impl<S> Layer<S> for SignedContextLayer {
    type Service = SignedContextService<S>;

    fn layer(&self, service: S) -> Self::Service {
        SignedContextService {
            inner: service,
            verifier: self.verifier.clone(),
            allow_anonymous: self.allow_anonymous,
        }
    }
}

#[derive(Clone)]
pub struct SignedContextService<S> {
    inner: S,
    verifier: SignedContextVerifier,
    allow_anonymous: bool,
}

impl<S> tonic::server::NamedService for SignedContextService<S>
where
    S: tonic::server::NamedService,
{
    const NAME: &'static str = S::NAME;
}

impl<S> Service<HttpRequest<Body>> for SignedContextService<S>
where
    S: Service<HttpRequest<Body>, Response = HttpResponse<Body>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = HttpResponse<Body>;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: HttpRequest<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let verifier = self.verifier.clone();
        let allow_anonymous = self.allow_anonymous;

        Box::pin(async move {
            let mut supplied = req.headers().get_all(SIGNED_CONTEXT_HEADER).iter();
            let first = supplied.next();
            let ctx = match (first, supplied.next()) {
                (Some(_), Some(_)) => Err(Status::unauthenticated("duplicate signed context")),
                (Some(header), None) => header
                    .to_str()
                    .map_err(|_| Status::unauthenticated("invalid signed context"))
                    .and_then(|token| {
                        verifier
                            .verify(token)
                            .map(|claims| signed_context(&claims, token))
                            .map_err(|_| Status::unauthenticated("invalid signed context"))
                    }),
                (None, None) if allow_anonymous => Ok(anonymous_context()),
                (None, None) => Err(Status::unauthenticated("signed context required")),
                _ => Err(Status::unauthenticated("invalid signed context")),
            };
            let ctx = match ctx {
                Ok(ctx) => ctx,
                Err(status) => return Ok(status.into_http()),
            };

            // Even handlers reading raw metadata cannot mistake caller-supplied
            // legacy identity fields for authenticated identity.
            let untrusted: Vec<_> = req
                .headers()
                .keys()
                .filter(|name| {
                    let name = name.as_str();
                    matches!(
                        name,
                        "x-user-id"
                            | "x-tenant-id"
                            | "x-device-id"
                            | "x-session-id"
                            | "x-request-id"
                            | "x-credential-id"
                    ) || name.starts_with("x-actor-")
                })
                .cloned()
                .collect();
            for name in untrusted {
                req.headers_mut().remove(name);
            }
            req.extensions_mut().insert(ctx);
            inner.call(req).await
        })
    }
}

fn signed_context(claims: &SignedContextClaims, token: &str) -> Ctx {
    let actor_type = match claims.actor_type {
        SignedActorType::User => ActorType::User,
        SignedActorType::Service => ActorType::Service,
        SignedActorType::TenantAdmin => ActorType::TenantAdmin,
        SignedActorType::System => ActorType::System,
        SignedActorType::Guest => ActorType::Guest,
    };
    let mut actor = ActorContext::new(&claims.actor_id).with_type(actor_type);
    for role in &claims.roles {
        actor = actor.with_role(role);
    }
    if let Some(credential_id) = &claims.credential_id {
        actor = actor.with_attribute("credential_id", credential_id);
    }
    let mut ctx = Context::with_request_id(&claims.request_id)
        .with_tenant_id(&claims.tenant_id)
        .with_actor(actor);
    if claims.actor_type == SignedActorType::User {
        ctx = ctx.with_user_id(&claims.actor_id);
    }
    Arc::new(ctx.insert_data(VerifiedContext::signed(claims.clone(), token.to_owned())))
}

fn anonymous_context() -> Ctx {
    Arc::new(
        Context::with_request_id(Uuid::new_v4().to_string())
            .with_actor(ActorContext::new("anonymous").with_type(ActorType::Guest))
            .insert_data(VerifiedContext::anonymous()),
    )
}

#[cfg(test)]
mod tests {
    use std::future::{Ready, ready};
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::SigningKey;
    use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use flare_core_infra::auth::{
        SignedContextIdentity, SignedContextSigner, SignedContextVerifier,
    };
    use tonic::Request;

    use super::*;
    use crate::grpc::utils::require_ctx_from_request;

    #[derive(Clone)]
    struct ObservingService(Arc<Mutex<Option<(Ctx, bool)>>>);

    impl Service<HttpRequest<Body>> for ObservingService {
        type Response = HttpResponse<Body>;
        type Error = Infallible;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: HttpRequest<Body>) -> Self::Future {
            let ctx = req.extensions().get::<Ctx>().cloned().unwrap();
            let bare_identity_survived = req.headers().contains_key("x-user-id")
                || req.headers().contains_key("x-tenant-id")
                || req.headers().contains_key("x-actor-roles");
            *self.0.lock().unwrap() = Some((ctx, bare_identity_survived));
            ready(Ok(HttpResponse::new(Body::empty())))
        }
    }

    fn keys() -> (SignedContextSigner, SignedContextVerifier) {
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

    #[tokio::test]
    async fn signed_context_ignores_forged_legacy_metadata() {
        let (signer, verifier) = keys();
        let token = signer
            .sign(&SignedContextIdentity::new(
                SignedActorType::User,
                "real-user",
                "real-tenant",
                "request-1",
            ))
            .unwrap();
        let seen = Arc::new(Mutex::new(None));
        let mut service = SignedContextLayer::new(verifier).layer(ObservingService(seen.clone()));
        let req = HttpRequest::builder()
            .header(SIGNED_CONTEXT_HEADER, &token)
            .header("x-user-id", "attacker")
            .header("x-tenant-id", "other-tenant")
            .header("x-actor-roles", "control")
            .body(Body::empty())
            .unwrap();
        service.call(req).await.unwrap();
        let (ctx, had_legacy_headers) = seen.lock().unwrap().take().unwrap();
        assert!(!had_legacy_headers);
        assert_eq!(ctx.user_id(), Some("real-user"));
        assert_eq!(ctx.tenant_id(), Some("real-tenant"));
        assert_eq!(ctx.actor().unwrap().actor_id(), "real-user");
        assert!(ctx.actor().unwrap().roles().is_empty());
        assert_eq!(
            ctx.get_data::<VerifiedContext>().unwrap().token(),
            Some(token.as_str())
        );

        let mut tonic_req = Request::new(());
        tonic_req.extensions_mut().insert(ctx);
        tonic_req
            .metadata_mut()
            .insert("x-user-id", "attacker".parse().unwrap());
        tonic_req
            .metadata_mut()
            .insert("x-tenant-id", "other-tenant".parse().unwrap());
        let merged = require_ctx_from_request(&tonic_req).unwrap();
        assert_eq!(merged.user_id(), Some("real-user"));
        assert_eq!(merged.tenant_id(), Some("real-tenant"));
    }

    #[tokio::test]
    async fn missing_and_invalid_tokens_never_reach_protected_service() {
        let (_, verifier) = keys();
        let seen = Arc::new(Mutex::new(None));
        let mut service =
            SignedContextLayer::new(verifier.clone()).layer(ObservingService(seen.clone()));
        let missing = service.call(HttpRequest::new(Body::empty())).await.unwrap();
        assert_eq!(missing.headers().get("grpc-status").unwrap(), "16");
        assert!(seen.lock().unwrap().is_none());

        let mut duplicate = HttpRequest::new(Body::empty());
        duplicate
            .headers_mut()
            .append(SIGNED_CONTEXT_HEADER, "forged".parse().unwrap());
        duplicate
            .headers_mut()
            .append(SIGNED_CONTEXT_HEADER, "also-forged".parse().unwrap());
        assert_eq!(
            service
                .call(duplicate)
                .await
                .unwrap()
                .headers()
                .get("grpc-status")
                .unwrap(),
            "16"
        );
        assert!(seen.lock().unwrap().is_none());

        let invalid = HttpRequest::builder()
            .header(SIGNED_CONTEXT_HEADER, "forged")
            .body(Body::empty())
            .unwrap();
        let rejected = service.call(invalid).await.unwrap();
        assert_eq!(rejected.headers().get("grpc-status").unwrap(), "16");
        assert!(seen.lock().unwrap().is_none());

        let mut public = SignedContextLayer::new(verifier)
            .allow_anonymous()
            .layer(ObservingService(seen.clone()));
        let invalid = HttpRequest::builder()
            .header(SIGNED_CONTEXT_HEADER, "forged")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            public
                .call(invalid)
                .await
                .unwrap()
                .headers()
                .get("grpc-status")
                .unwrap(),
            "16"
        );
        assert!(seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn anonymous_marker_blocks_metadata_backfill() {
        let (_, verifier) = keys();
        let seen = Arc::new(Mutex::new(None));
        let mut service = SignedContextLayer::new(verifier)
            .allow_anonymous()
            .layer(ObservingService(seen.clone()));
        let req = HttpRequest::builder()
            .header("x-user-id", "attacker")
            .header("x-tenant-id", "other-tenant")
            .body(Body::empty())
            .unwrap();
        service.call(req).await.unwrap();
        let (ctx, had_legacy_headers) = seen.lock().unwrap().take().unwrap();
        assert!(!had_legacy_headers);
        assert_eq!(ctx.user_id(), None);
        assert_eq!(ctx.tenant_id(), None);
        assert_eq!(ctx.actor().unwrap().actor_id(), "anonymous");
        assert_eq!(ctx.actor().unwrap().actor_type, ActorType::Guest);
        assert!(ctx.get_data::<VerifiedContext>().unwrap().token().is_none());
        let mut tonic_req = Request::new(());
        tonic_req.extensions_mut().insert(ctx);
        tonic_req
            .metadata_mut()
            .insert("x-user-id", "attacker".parse().unwrap());
        tonic_req
            .metadata_mut()
            .insert("x-tenant-id", "other-tenant".parse().unwrap());
        let merged = require_ctx_from_request(&tonic_req).unwrap();
        assert_eq!(merged.user_id(), None);
        assert_eq!(merged.tenant_id(), None);
    }

    #[test]
    fn legacy_metadata_path_does_not_gain_a_verified_marker() {
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("x-user-id", "legacy-user".parse().unwrap());
        req.metadata_mut()
            .insert("x-tenant-id", "legacy-tenant".parse().unwrap());
        let ctx = require_ctx_from_request(&req).unwrap();
        assert_eq!(ctx.user_id(), Some("legacy-user"));
        assert_eq!(ctx.tenant_id(), Some("legacy-tenant"));
        assert!(ctx.get_data::<VerifiedContext>().is_none());
    }

    #[test]
    fn method_level_verifier_rejects_bare_identity_and_duplicate_token() {
        let (signer, verifier) = keys();
        let token = signer
            .sign(&SignedContextIdentity::new(
                SignedActorType::User,
                "alice",
                "tenant-a",
                "req-1",
            ))
            .unwrap();
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("x-user-id", "attacker".parse().unwrap());
        assert_eq!(
            verify_signed_context_metadata(req.metadata(), &verifier)
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
        req.metadata_mut()
            .append(SIGNED_CONTEXT_HEADER, token.parse().unwrap());
        let ctx = verify_signed_context_metadata(req.metadata(), &verifier).unwrap();
        assert_eq!(ctx.user_id(), Some("alice"));
        assert_eq!(ctx.tenant_id(), Some("tenant-a"));
        assert_eq!(
            ctx.get_data::<VerifiedContext>().unwrap().token(),
            Some(token.as_str())
        );
        req.metadata_mut()
            .append(SIGNED_CONTEXT_HEADER, token.parse().unwrap());
        assert_eq!(
            verify_signed_context_metadata(req.metadata(), &verifier)
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
    }
}
