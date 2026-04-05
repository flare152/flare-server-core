//! gRPC 服务端模块

#[cfg(feature = "grpc-interceptor")]
pub mod interceptor;

#[cfg(feature = "grpc-middleware")]
pub mod middleware;

#[cfg(feature = "grpc-interceptor")]
pub use interceptor::{
    AuthInterceptor, CompositeInterceptor, LoggingInterceptor, TraceInfo, TracingInterceptor,
    extract_trace_info,
};

#[cfg(feature = "grpc-middleware")]
pub use middleware::{
    ContextLayer, ContextService, RateLimitLayer, RetryLayer, TimeoutLayer, extract_actor_id,
    extract_context, extract_request_id, extract_tenant_id, extract_user_id, get_context,
    require_actor_id, require_request_id, require_tenant_id, require_user_id,
};
