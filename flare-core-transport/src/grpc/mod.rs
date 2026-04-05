//! gRPC 模块
//!
//! 提供 gRPC 客户端、服务端、拦截器、中间件等能力

// ===== 工具函数 (基础,不依赖其他模块) =====
pub mod utils;

// ===== 客户端 =====
pub mod client;

// ===== 服务端 =====
pub mod server;

// ===== 拦截器 =====
pub mod interceptor;

// ===== 中间件 =====
pub mod middleware;

// ===== 健康检查 =====
pub mod health;

// ===== 重试策略 =====
pub mod retry;

// ===== 重试策略实现 =====
pub mod exponential;
pub mod fixed;

// ===== 运行时适配器 =====
pub mod adapter;

// Re-exports
pub use adapter::{GrpcAdapter, GrpcAdapterBuilder};
pub use exponential::ExponentialBackoffPolicy;
pub use fixed::FixedRetryPolicy;

use std::time::Duration;

/// 重试策略 trait
pub trait RetryPolicy {
    fn should_retry(&self, attempt: usize, error: &tonic::Status) -> bool;
    fn backoff_duration(&self, attempt: usize) -> Duration;
    fn max_attempts(&self) -> usize;
}
