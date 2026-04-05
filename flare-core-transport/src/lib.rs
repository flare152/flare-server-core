//! Flare Core Transport - 传输层
//!
//! 提供 gRPC、HTTP、服务发现等传输层能力
//!
//! # Features
//!
//! - `grpc`: gRPC 支持 (可选)
//! - `http`: HTTP 支持 (可选)
//! - `discovery`: 服务发现支持 (可选)
//!
//! # 示例
//!
//! ```toml
//! [dependencies]
//! flare-core-transport = { version = "0.2", features = ["grpc", "http"] }
//! ```

// ===== gRPC (可选) =====
#[cfg(feature = "grpc")]
pub mod grpc;

// ===== HTTP (可选) =====
#[cfg(feature = "http")]
pub mod http;

// ===== 服务发现 (可选) =====
#[cfg(feature = "discovery")]
pub mod discovery;

// Re-exports - gRPC
#[cfg(feature = "grpc")]
pub use grpc::{
    ExponentialBackoffPolicy, FixedRetryPolicy, RetryPolicy, client::*, health::*, interceptor::*,
    middleware::*, retry::*, utils::*,
};

#[cfg(feature = "http")]
pub use http::{middleware::*, response::*};

#[cfg(feature = "discovery")]
pub use discovery::{
    DiscoveryBackend, DiscoveryConfig, DiscoveryFactory, ServiceDiscover, ServiceInstance,
};
