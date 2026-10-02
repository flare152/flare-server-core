//! Signed-first identity boundary for services still migrating from legacy
//! metadata identity.
//!
//! `SignedContextLayer` admits only `x-flare-ctx`. Services that still have
//! callers sending bare `x-tenant-id` / `x-user-id` use this layer instead:
//!
//! - A single `x-flare-ctx` is verified and becomes the only identity. Bare
//!   identity metadata that disagrees with it rejects the call, so a caller
//!   cannot pair a valid token with another tenant or user. The bare identity
//!   headers are then stripped, exactly as `SignedContextLayer` does.
//! - Without a token the call fails when `require_signed` is set; otherwise it
//!   keeps the legacy metadata context (what `ContextLayer::allow_missing` gave).
//! - Methods marked exempt authenticate by other means (for example a forwarded
//!   admin bearer checked by the handler) and keep the legacy context unchanged.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use flare_core_base::context::{ActorType, Context, Ctx};
use flare_core_infra::auth::{SignedActorType, SignedContextClaims, SignedContextVerifier};
use http::{HeaderMap, Request as HttpRequest, Response as HttpResponse};
use tonic::Status;
use tonic::body::Body;
use tonic::metadata::{MetadataMap, MetadataValue};
use tower::{Layer, Service};

use crate::grpc::utils::decode_context_from_metadata;

use super::signed_context::{
    SIGNED_CONTEXT_HEADER, VerifiedContext, verify_signed_context_metadata,
};

/// Bare identity headers a signed call may repeat only with the signed values.
const TENANT_HEADER: &str = "x-tenant-id";
const USER_HEADER: &str = "x-user-id";
const ACTOR_ID_HEADER: &str = "x-actor-id";
const ACTOR_TYPE_HEADER: &str = "x-actor-type";
const ACTOR_ROLES_HEADER: &str = "x-actor-roles";

/// Actor types a read service accepts by default: a user on whose behalf a
/// gateway signs, a tenant administrator, and a Cell service.
const DEFAULT_ALLOWED: [SignedActorType; 3] = [
    SignedActorType::User,
    SignedActorType::TenantAdmin,
    SignedActorType::Service,
];

#[derive(Clone)]
pub struct SignedFirstContextLayer {
    verifier: Option<SignedContextVerifier>,
    require_signed: bool,
    allowed: Arc<[SignedActorType]>,
    exempt_methods: Arc<BTreeSet<String>>,
}

impl SignedFirstContextLayer {
    /// Signed context wins when present; unsigned calls keep the legacy context.
    ///
    /// Without a verifier the service cannot check tokens. While signing is not
    /// required it then behaves exactly like `ContextLayer::allow_missing`: a
    /// supplied token is ignored and the bare metadata decides, as before this
    /// layer, so a receiver deployed without the public key keeps serving callers
    /// that already sign. With signing required every call fails UNAVAILABLE.
    pub fn new(verifier: Option<SignedContextVerifier>) -> Self {
        Self {
            verifier,
            require_signed: false,
            allowed: Arc::from(DEFAULT_ALLOWED.as_slice()),
            exempt_methods: Arc::new(BTreeSet::new()),
        }
    }

    /// Refuse calls without `x-flare-ctx` (UNAUTHENTICATED; UNAVAILABLE when no
    /// verifier is configured).
    pub fn require_signed(mut self, require: bool) -> Self {
        self.require_signed = require;
        self
    }

    /// Replace the accepted actor types (PERMISSION_DENIED for the others).
    pub fn allow_actor_types(mut self, types: impl IntoIterator<Item = SignedActorType>) -> Self {
        let mut allowed: Vec<SignedActorType> = Vec::new();
        for actor_type in types {
            if !allowed.contains(&actor_type) {
                allowed.push(actor_type);
            }
        }
        self.allowed = Arc::from(allowed);
        self
    }

    /// Full gRPC path (`/package.Service/Method`) authenticated by its handler
    /// instead; it keeps the legacy metadata context and is not checked here.
    pub fn exempt_method(mut self, path: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.exempt_methods).insert(path.into());
        self
    }

    pub fn requires_signed(&self) -> bool {
        self.require_signed
    }

    pub fn layer<S>(&self, service: S) -> <Self as Layer<S>>::Service
    where
        Self: Layer<S>,
    {
        Layer::layer(self, service)
    }
}

impl<S> Layer<S> for SignedFirstContextLayer {
    type Service = SignedFirstContextService<S>;

    fn layer(&self, service: S) -> Self::Service {
        SignedFirstContextService {
            inner: service,
            policy: self.clone(),
        }
    }
}

#[derive(Clone)]
pub struct SignedFirstContextService<S> {
    inner: S,
    policy: SignedFirstContextLayer,
}

impl<S> tonic::server::NamedService for SignedFirstContextService<S>
where
    S: tonic::server::NamedService,
{
    const NAME: &'static str = S::NAME;
}

impl<S> Service<HttpRequest<Body>> for SignedFirstContextService<S>
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
        let policy = self.policy.clone();

        Box::pin(async move {
            if policy.exempt_methods.contains(req.uri().path()) {
                insert_legacy_context(&mut req);
                return inner.call(req).await;
            }
            match admit(&policy, req.headers()) {
                Ok(Some(ctx)) => {
                    strip_bare_identity(req.headers_mut());
                    req.extensions_mut().insert(ctx);
                }
                Ok(None) => insert_legacy_context(&mut req),
                Err(status) => return Ok(status.into_http()),
            }
            inner.call(req).await
        })
    }
}

/// `Ok(Some)` for a verified call, `Ok(None)` for an admitted legacy call.
fn admit(policy: &SignedFirstContextLayer, headers: &HeaderMap) -> Result<Option<Ctx>, Status> {
    let Some(verifier) = policy.verifier.as_ref() else {
        if policy.require_signed {
            return Err(Status::unavailable(
                "signed context verifier not configured",
            ));
        }
        return Ok(None);
    };
    let mut supplied = headers.get_all(SIGNED_CONTEXT_HEADER).iter();
    let token = match (supplied.next(), supplied.next()) {
        (None, _) if !policy.require_signed => return Ok(None),
        (None, _) => return Err(Status::unauthenticated("signed context required")),
        (Some(_), Some(_)) => return Err(Status::unauthenticated("duplicate signed context")),
        (Some(token), None) => token,
    };
    let token = token
        .to_str()
        .map_err(|_| Status::unauthenticated("invalid signed context"))?;
    let value = token
        .parse::<MetadataValue<_>>()
        .map_err(|_| Status::unauthenticated("invalid signed context"))?;
    let mut carrier = MetadataMap::new();
    carrier.insert(SIGNED_CONTEXT_HEADER, value);
    let ctx = verify_signed_context_metadata(&carrier, verifier)?;
    let claims = ctx
        .get_data::<VerifiedContext>()
        .and_then(VerifiedContext::claims)
        .ok_or_else(|| Status::unauthenticated("invalid signed context"))?;
    if !policy.allowed.contains(&claims.actor_type) {
        return Err(Status::permission_denied(
            "signed actor type not permitted on this service",
        ));
    }
    if bare_identity_conflicts(headers, claims) {
        return Err(Status::permission_denied(
            "unsigned identity metadata conflicts with the signed context",
        ));
    }
    Ok(Some(ctx))
}

fn signed_actor_type(actor_type: SignedActorType) -> ActorType {
    match actor_type {
        SignedActorType::User => ActorType::User,
        SignedActorType::Service => ActorType::Service,
        SignedActorType::TenantAdmin => ActorType::TenantAdmin,
        SignedActorType::System => ActorType::System,
        SignedActorType::Guest => ActorType::Guest,
    }
}

/// Every bare identity value a caller sends next to the token must equal the
/// signed one: tenant, user (only `user` and `tenant_admin` actors have one),
/// actor id and type, and the role set. A value that is not valid text is a
/// conflict too. Absent headers are fine; device, session and request ids are
/// not identity and are only stripped.
fn bare_identity_conflicts(headers: &HeaderMap, claims: &SignedContextClaims) -> bool {
    let signed_user = matches!(
        claims.actor_type,
        SignedActorType::User | SignedActorType::TenantAdmin
    )
    .then_some(claims.actor_id.as_str());
    let signed_type = i32::from(signed_actor_type(claims.actor_type)).to_string();
    let mismatch = |name: &str, expected: Option<&str>| {
        headers
            .get_all(name)
            .iter()
            .any(|value| match value.to_str() {
                Ok(value) => {
                    let value = value.trim();
                    !value.is_empty() && Some(value) != expected
                }
                Err(_) => true,
            })
    };
    if mismatch(TENANT_HEADER, Some(claims.tenant_id.as_str()))
        || mismatch(USER_HEADER, signed_user)
        || mismatch(ACTOR_ID_HEADER, Some(claims.actor_id.as_str()))
        || mismatch(ACTOR_TYPE_HEADER, Some(signed_type.as_str()))
    {
        return true;
    }
    let mut bare_roles = BTreeSet::new();
    for value in headers.get_all(ACTOR_ROLES_HEADER) {
        match value.to_str() {
            Ok(role) if !role.trim().is_empty() => {
                bare_roles.insert(role.trim());
            }
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    let signed_roles: BTreeSet<&str> = claims
        .roles
        .iter()
        .map(|role| role.trim())
        .filter(|role| !role.is_empty())
        .collect();
    !bare_roles.is_empty() && bare_roles != signed_roles
}

/// Same header set `SignedContextLayer` removes, so handlers reading raw
/// metadata cannot mistake a caller's bare values for the verified identity.
fn strip_bare_identity(headers: &mut HeaderMap) {
    let names: Vec<_> = headers
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
    for name in names {
        headers.remove(name);
    }
}

/// The context `ContextLayer::allow_missing` produced: one an outer layer
/// already set, else the metadata decoding.
fn insert_legacy_context(req: &mut HttpRequest<Body>) {
    if req.extensions().get::<Ctx>().is_some() {
        return;
    }
    let metadata = MetadataMap::from_headers(req.headers().clone());
    let ctx = decode_context_from_metadata(&metadata).unwrap_or_else(|| Arc::new(Context::root()));
    req.extensions_mut().insert(ctx);
}

#[cfg(test)]
mod tests {
    use std::future::{Ready, ready};
    use std::sync::Mutex;

    use ed25519_dalek::SigningKey;
    use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use flare_core_infra::auth::{SignedContextIdentity, SignedContextSigner};
    use tonic::Request;

    use super::*;
    use crate::grpc::utils::require_ctx_from_request;

    type Seen = Arc<Mutex<Option<(Ctx, bool)>>>;

    #[derive(Clone)]
    struct Observe(Seen);

    impl Service<HttpRequest<Body>> for Observe {
        type Response = HttpResponse<Body>;
        type Error = Infallible;
        type Future = Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: HttpRequest<Body>) -> Self::Future {
            let ctx = req.extensions().get::<Ctx>().cloned().unwrap();
            let bare = req.headers().contains_key("x-user-id")
                || req.headers().contains_key("x-tenant-id")
                || req.headers().contains_key("x-actor-type");
            *self.0.lock().unwrap() = Some((ctx, bare));
            ready(Ok(HttpResponse::new(Body::empty())))
        }
    }

    fn keys(seed: u8) -> (SignedContextSigner, SignedContextVerifier) {
        let key = SigningKey::from_bytes(&[seed; 32]);
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

    fn sign(signer: &SignedContextSigner, actor_type: SignedActorType, actor: &str) -> String {
        signer
            .sign(&SignedContextIdentity::new(
                actor_type, actor, "tenant-a", "req-1",
            ))
            .unwrap()
    }

    fn call(
        layer: &SignedFirstContextLayer,
        path: &str,
        headers: &[(&str, &str)],
    ) -> (Option<String>, Option<(Ctx, bool)>) {
        let seen: Seen = Arc::new(Mutex::new(None));
        let mut service = layer.layer(Observe(seen.clone()));
        let mut builder = HttpRequest::builder().uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response =
            tokio_test::block_on(service.call(builder.body(Body::empty()).unwrap())).unwrap();
        let status = response
            .headers()
            .get("grpc-status")
            .map(|code| code.to_str().unwrap().to_owned());
        let observed = seen.lock().unwrap().take();
        (status, observed)
    }

    const READ: &str = "/flare.test.Reads/List";

    #[test]
    fn signed_identity_wins_and_bare_identity_is_stripped() {
        let (signer, verifier) = keys(61);
        let token = sign(&signer, SignedActorType::User, "alice");
        let layer = SignedFirstContextLayer::new(Some(verifier)).require_signed(true);
        // Agreeing bare metadata (what a forwarding caller encodes) is accepted.
        let (status, seen) = call(
            &layer,
            READ,
            &[
                (SIGNED_CONTEXT_HEADER, &token),
                ("x-tenant-id", "tenant-a"),
                ("x-user-id", "alice"),
                ("x-actor-id", "alice"),
                ("x-actor-type", "1"),
                ("x-device-id", "phone"),
            ],
        );
        assert_eq!(status, None);
        let (ctx, bare_survived) = seen.unwrap();
        assert!(!bare_survived);
        assert_eq!(ctx.tenant_id(), Some("tenant-a"));
        assert_eq!(ctx.user_id(), Some("alice"));
        let verified = ctx.get_data::<VerifiedContext>().unwrap();
        assert_eq!(verified.token(), Some(token.as_str()));

        // Metadata backfill in the handler cannot change it either.
        let mut tonic_req = Request::new(());
        tonic_req.extensions_mut().insert(ctx);
        tonic_req
            .metadata_mut()
            .insert("x-user-id", "mallory".parse().unwrap());
        let merged = require_ctx_from_request(&tonic_req).unwrap();
        assert_eq!(merged.user_id(), Some("alice"));
    }

    #[test]
    fn bare_identity_disagreeing_with_the_token_is_refused() {
        let (signer, verifier) = keys(62);
        let user = sign(&signer, SignedActorType::User, "alice");
        let service = sign(&signer, SignedActorType::Service, "flare-orchestrator");
        let layer = SignedFirstContextLayer::new(Some(verifier));
        for (token, name, value) in [
            (&user, "x-tenant-id", "tenant-b"),
            (&user, "x-user-id", "bob"),
            (&user, "x-actor-id", "bob"),
            (&user, "x-actor-type", "2"),
            (&user, "x-actor-type", "user"),
            (&user, "x-actor-roles", "control"),
            // A service is nobody's user: a bare user id next to it is a claim the
            // token does not make.
            (&service, "x-user-id", "alice"),
            (&service, "x-actor-type", "1"),
        ] {
            let (status, seen) = call(
                &layer,
                READ,
                &[(SIGNED_CONTEXT_HEADER, token), (name, value)],
            );
            assert_eq!(status.as_deref(), Some("7"), "{name}={value}");
            assert!(seen.is_none(), "{name}={value}");
        }
        // The same service token with agreeing metadata passes and has no user.
        let (status, seen) = call(
            &layer,
            READ,
            &[
                (SIGNED_CONTEXT_HEADER, &service),
                ("x-tenant-id", "tenant-a"),
                ("x-actor-id", "flare-orchestrator"),
                ("x-actor-type", "2"),
            ],
        );
        assert_eq!(status, None);
        let (ctx, _) = seen.unwrap();
        assert_eq!(ctx.user_id(), None);
        assert_eq!(ctx.actor().unwrap().actor_type, ActorType::Service);
    }

    #[test]
    fn unsigned_calls_are_refused_only_when_required() {
        let (signer, verifier) = keys(63);
        let legacy = [("x-tenant-id", "tenant-b"), ("x-user-id", "bob")];

        let optional = SignedFirstContextLayer::new(Some(verifier.clone()));
        let (status, seen) = call(&optional, READ, &legacy);
        assert_eq!(status, None);
        let (ctx, _) = seen.unwrap();
        assert_eq!(ctx.user_id(), Some("bob"));
        assert!(ctx.get_data::<VerifiedContext>().is_none());

        let required = optional.clone().require_signed(true);
        let (status, seen) = call(&required, READ, &legacy);
        assert_eq!(status.as_deref(), Some("16"));
        assert!(seen.is_none());

        // Forged, foreign-key, duplicate and wrong-type tokens never pass.
        let (other_signer, _) = keys(64);
        let foreign = sign(&other_signer, SignedActorType::User, "alice");
        let good = sign(&signer, SignedActorType::User, "alice");
        let guest = sign(&signer, SignedActorType::Guest, "anonymous");
        for layer in [&optional, &required] {
            for headers in [
                vec![(SIGNED_CONTEXT_HEADER, "forged")],
                vec![(SIGNED_CONTEXT_HEADER, foreign.as_str())],
                vec![
                    (SIGNED_CONTEXT_HEADER, good.as_str()),
                    (SIGNED_CONTEXT_HEADER, good.as_str()),
                ],
            ] {
                let (status, seen) = call(layer, READ, &headers);
                assert_eq!(status.as_deref(), Some("16"));
                assert!(seen.is_none());
            }
            let (status, seen) = call(layer, READ, &[(SIGNED_CONTEXT_HEADER, &guest)]);
            assert_eq!(status.as_deref(), Some("7"));
            assert!(seen.is_none());
        }
    }

    #[test]
    fn without_a_verifier_required_calls_fail_closed_and_optional_ones_stay_legacy() {
        let (signer, _) = keys(65);
        let token = sign(&signer, SignedActorType::User, "alice");
        let required = SignedFirstContextLayer::new(None).require_signed(true);
        for headers in [vec![(SIGNED_CONTEXT_HEADER, token.as_str())], vec![]] {
            let (status, seen) = call(&required, READ, &headers);
            assert_eq!(status.as_deref(), Some("14"));
            assert!(seen.is_none());
        }
        // Not required: the token cannot be checked, so it confers nothing and the
        // bare metadata decides, exactly as without this layer.
        let optional = SignedFirstContextLayer::new(None);
        let (status, seen) = call(
            &optional,
            READ,
            &[(SIGNED_CONTEXT_HEADER, &token), ("x-tenant-id", "tenant-a")],
        );
        assert_eq!(status, None);
        let (ctx, _) = seen.unwrap();
        assert_eq!(ctx.tenant_id(), Some("tenant-a"));
        assert_eq!(ctx.user_id(), None);
        assert!(ctx.get_data::<VerifiedContext>().is_none());
    }

    #[test]
    fn exempt_methods_and_actor_allow_list() {
        let (signer, verifier) = keys(66);
        let admin = "/flare.test.Reads/AdminSearch";
        let layer = SignedFirstContextLayer::new(Some(verifier))
            .require_signed(true)
            .exempt_method(admin)
            .allow_actor_types([SignedActorType::User]);
        // The exempt method keeps its legacy context and is not asked for a token.
        let (status, seen) = call(&layer, admin, &[("x-tenant-id", "tenant-a")]);
        assert_eq!(status, None);
        assert_eq!(seen.unwrap().0.tenant_id(), Some("tenant-a"));
        // Other methods are still required to carry an accepted actor type.
        let (status, _) = call(&layer, READ, &[]);
        assert_eq!(status.as_deref(), Some("16"));
        let service = sign(&signer, SignedActorType::Service, "flare-orchestrator");
        let (status, _) = call(&layer, READ, &[(SIGNED_CONTEXT_HEADER, &service)]);
        assert_eq!(status.as_deref(), Some("7"));
        let user = sign(&signer, SignedActorType::User, "alice");
        let (status, _) = call(&layer, READ, &[(SIGNED_CONTEXT_HEADER, &user)]);
        assert_eq!(status, None);
    }
}
