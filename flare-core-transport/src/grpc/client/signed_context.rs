//! Attach a short-lived, already-issued internal context token to gRPC calls.

use std::sync::Arc;

use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::{Request, Status};

use flare_core_base::context::Context;

use crate::grpc::middleware::signed_context::{SIGNED_CONTEXT_HEADER, VerifiedContext};

pub fn set_signed_context_token<T>(request: &mut Request<T>, token: &str) -> Result<(), Status> {
    let value = token
        .parse::<MetadataValue<Ascii>>()
        .map_err(|_| Status::invalid_argument("invalid signed context token encoding"))?;
    request.metadata_mut().insert(SIGNED_CONTEXT_HEADER, value);
    Ok(())
}

/// Forward the verified inbound token `ctx` carries (`VerifiedContext::token`)
/// unchanged, so the next service sees the same signed identity. Returns
/// `Ok(false)`, leaving the request as it is, when the context was not built
/// from a verified token (legacy metadata or explicitly anonymous).
///
/// Pair it with `request_with_context(payload, ctx)` on the same verified
/// context: the bare metadata that encodes is then the signed identity, which a
/// signed-first receiver accepts. A context derived with another tenant or user
/// keeps the original token, and such a receiver refuses the disagreement.
pub fn forward_signed_context<T>(request: &mut Request<T>, ctx: &Context) -> Result<bool, Status> {
    match ctx
        .get_data::<VerifiedContext>()
        .and_then(VerifiedContext::token)
    {
        Some(token) => set_signed_context_token(request, token).map(|()| true),
        None => Ok(false),
    }
}

/// Invokes the provider per RPC so long-lived clients cannot retain an expired token.
#[derive(Clone)]
pub struct SignedContextInterceptor {
    token_provider: Arc<dyn Fn() -> Result<String, Status> + Send + Sync>,
}

impl SignedContextInterceptor {
    pub fn new(
        token_provider: impl Fn() -> Result<String, Status> + Send + Sync + 'static,
    ) -> Self {
        Self {
            token_provider: Arc::new(token_provider),
        }
    }
}

impl Interceptor for SignedContextInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let token = (self.token_provider)()?;
        set_signed_context_token(&mut request, &token)?;
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn interceptor_requests_a_fresh_token_for_each_rpc() {
        let issued = Arc::new(AtomicUsize::new(0));
        let count = issued.clone();
        let mut interceptor = SignedContextInterceptor::new(move || {
            let next = count.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(format!("token-{next}"))
        });
        let first = interceptor.call(Request::new(())).unwrap();
        let second = interceptor.call(Request::new(())).unwrap();
        assert_eq!(
            first.metadata().get(SIGNED_CONTEXT_HEADER).unwrap(),
            "token-1"
        );
        assert_eq!(
            second.metadata().get(SIGNED_CONTEXT_HEADER).unwrap(),
            "token-2"
        );
        assert!(set_signed_context_token(&mut Request::new(()), "\n").is_err());
    }

    #[test]
    fn only_a_verified_context_forwards_a_token() {
        use ed25519_dalek::SigningKey;
        use ed25519_dalek::pkcs8::{EncodePrivateKey, EncodePublicKey};
        use flare_core_infra::auth::{
            SignedActorType, SignedContextIdentity, SignedContextSigner, SignedContextVerifier,
        };

        use crate::grpc::middleware::signed_context::verify_signed_context_metadata;

        let key = SigningKey::from_bytes(&[71; 32]);
        let signer =
            SignedContextSigner::from_private_key(&key.to_pkcs8_pem(Default::default()).unwrap())
                .unwrap();
        let verifier = SignedContextVerifier::from_public_key(
            &key.verifying_key()
                .to_public_key_pem(Default::default())
                .unwrap(),
        )
        .unwrap();
        let token = signer
            .sign(&SignedContextIdentity::new(
                SignedActorType::User,
                "alice",
                "tenant-a",
                "req-1",
            ))
            .unwrap();
        let mut inbound = Request::new(());
        set_signed_context_token(&mut inbound, &token).unwrap();
        let verified = verify_signed_context_metadata(inbound.metadata(), &verifier).unwrap();

        let mut outbound = Request::new(());
        assert!(forward_signed_context(&mut outbound, &verified).unwrap());
        assert_eq!(
            outbound.metadata().get(SIGNED_CONTEXT_HEADER).unwrap(),
            token.as_str()
        );

        // A legacy context, even one naming the same user, forwards nothing.
        let legacy = Context::root()
            .with_tenant_id("tenant-a")
            .with_user_id("alice");
        let mut outbound = Request::new(());
        assert!(!forward_signed_context(&mut outbound, &legacy).unwrap());
        assert!(outbound.metadata().get(SIGNED_CONTEXT_HEADER).is_none());
    }
}
