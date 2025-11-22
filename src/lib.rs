//! Flare IM Server Core Library
//!
//! Provides complete gRPC infrastructure for server-side including interceptors, middleware,
//! error handling, service discovery, and more.

pub mod auth;
pub mod config;
pub mod error;
pub mod i18n;
pub mod discovery;
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

// 微服务运行时框架
pub mod runtime;

// Kafka 工具模块（可选）
#[cfg(feature = "kafka")]
pub mod kafka;

// Re-exports
pub use auth::{TokenClaims, TokenService};
pub use config::{Config, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig};
pub use error::{
    ErrorBuilder, ErrorCategory, ErrorCode, FlareError, FlareServerError, GrpcError, GrpcErrorExt,
    GrpcResult, LocalizedError, Result, ServerError,
};
pub use i18n::{I18n, default_en_us_translations, default_zh_cn_translations};

// 统一服务发现模块（新版 API）
pub use discovery::{
    DiscoveryBackend, DiscoveryConfig, DiscoveryFactory, ServiceDiscover, ServiceDiscoverUpdater,
    ServiceClient, ChannelService, ServiceInstance, ServiceRegistry, NamespaceConfig,
    VersionConfig, TagFilter, InstanceMetadata, BackendType, LoadBalanceStrategy, HealthCheckConfig,
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

// 运行时框架 re-exports
pub use runtime::ServiceRuntime;
pub use runtime::task::{Task, MessageConsumer, MessageConsumerTask, SpawnTask};

// Kafka 工具 re-exports（可选）
#[cfg(feature = "kafka")]
pub use kafka::{KafkaConsumerConfig, build_kafka_consumer, subscribe_and_wait_for_assignment};
