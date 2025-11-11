//! Flare IM Server Core Library
//!
//! Provides complete gRPC infrastructure for server-side including interceptors, middleware,
//! error handling, service discovery, and more.

pub mod auth;
pub mod config;
pub mod error;
pub mod i18n;
pub mod registry;
pub mod types;

// gRPC 基础功能模块
pub mod client;
pub mod health;
pub mod interceptor;
pub mod metrics;
pub mod middleware;
pub mod retry;
pub mod server;
pub mod utils;

// Re-exports
pub use auth::{TokenClaims, TokenService};
pub use config::{Config, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig};
pub use error::{
    ErrorBuilder, ErrorCategory, ErrorCode, FlareError, FlareServerError, GrpcError, GrpcErrorExt,
    GrpcResult, LocalizedError, Result, ServerError,
};
pub use i18n::{I18n, default_en_us_translations, default_zh_cn_translations};
pub use registry::{
    ConsulRegistry, EtcdRegistry, LoadBalanceStrategy, LoadBalancer, MeshRegistry, RegistryType,
    ServiceManager, ServiceRegistry, ServiceRegistryTrait, ServiceSelector, create_registry,
};
pub use types::{ServiceInfo, ServiceType};

// gRPC 相关 re-exports
pub use client::*;
pub use health::*;
pub use interceptor::*;
pub use metrics::*;
pub use middleware::*;
pub use retry::*;
pub use server::*;
pub use utils::*;
