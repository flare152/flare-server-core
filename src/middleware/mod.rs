//! gRPC 中间件模块
//!
//! 提供超时、限流、重试、上下文等中间件功能
//!
//! # 推荐使用
//!
//! **推荐使用 `ContextLayer`**，它统一处理所有上下文（Context、TenantContext、RequestContext），
//! 性能最优，代码最简洁。
//!
//! ```rust,no_run
//! use flare_server_core::middleware::ContextLayer;
//! use tonic::transport::Server;
//!
//! Server::builder()
//!     .layer(ContextLayer::new().allow_missing())
//!     .add_service(YourServiceServer::new(handler))
//!     .serve(addr)
//!     .await?;
//! ```

pub mod rate_limit;
pub mod context;
pub mod retry;
pub mod timeout;

pub use rate_limit::RateLimitLayer;
pub use retry::RetryLayer;
pub use timeout::TimeoutLayer;

// 推荐使用：统一的 Context 中间件
pub use context::{
    ContextLayer, ContextService, extract_context,
    // 向后兼容的便捷函数（所有旧的便捷函数都保留在这里）
    extract_tenant_context, extract_tenant_id, require_tenant_context, require_tenant_id,
    extract_request_context, extract_user_id, extract_actor_id, extract_request_id,
    require_request_context, require_user_id, require_actor_id, require_request_id,
};
