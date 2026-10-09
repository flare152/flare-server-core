//! gRPC 中间件
//!
//! 提供超时、限流、重试、Context 等中间件。
//!
//! ```rust,ignore
//! use flare_server_core::middleware::ContextLayer;
//! use tonic::transport::Server;
//!
//! Server::builder()
//!     .layer(ContextLayer::new().allow_missing())
//!     .add_service(YourServiceServer::new(handler))
//!     .serve(addr)
//!     .await?;
//! ```

pub mod context;
pub mod rate_limit;
pub mod retry;
pub mod signed_context;
pub mod signed_first_context;
pub mod timeout;

pub use context::{
    ContextLayer, ContextService, extract_actor_id, extract_context, extract_request_id,
    extract_tenant_id, extract_user_id, get_context, require_actor_id, require_request_id,
    require_tenant_id, require_user_id,
};
pub use rate_limit::RateLimitLayer;
pub use retry::RetryLayer;
pub use signed_context::{
    SIGNED_CONTEXT_HEADER, SignedContextLayer, SignedContextService, VerifiedContext,
    require_staff_permission, require_staff_permission_any, staff_claims,
};
pub use signed_first_context::{SignedFirstContextLayer, SignedFirstContextService};
pub use timeout::TimeoutLayer;
