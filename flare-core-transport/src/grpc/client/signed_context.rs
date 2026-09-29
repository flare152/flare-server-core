//! Attach a short-lived, already-issued internal context token to gRPC calls.

use std::sync::Arc;

use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::{Request, Status};

use crate::grpc::middleware::signed_context::SIGNED_CONTEXT_HEADER;

pub fn set_signed_context_token<T>(request: &mut Request<T>, token: &str) -> Result<(), Status> {
    let value = token
        .parse::<MetadataValue<Ascii>>()
        .map_err(|_| Status::invalid_argument("invalid signed context token encoding"))?;
    request.metadata_mut().insert(SIGNED_CONTEXT_HEADER, value);
    Ok(())
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
}
