//! Flare IM Server Core Library
//!
//! Provides complete gRPC infrastructure for server-side including interceptors, middleware,
//! error handling, service discovery, and more.

pub mod auth;
pub mod config;
pub mod context;
pub mod error;
pub mod event_bus;
pub mod i18n;
pub mod discovery;
pub mod kv;
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

// Kafka 由 event_bus 提供（feature kafka 时）：通用生产者/消费者 + 事件总线后端

// Re-exports
pub use auth::{TokenClaims, TokenService};
pub use config::{Config, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig};
pub use error::{
    ErrorBuilder, ErrorCategory, ErrorCode, FlareError, FlareServerError, GrpcError, GrpcErrorExt,
    GrpcResult, LocalizedError, Result, ServerError,
};
pub use i18n::{I18n, default_en_us_translations, default_zh_cn_translations};

// 统一服务发现模块
pub use discovery::{
    DiscoveryBackend, DiscoveryConfig, DiscoveryFactory, ServiceDiscover, ServiceDiscoverUpdater,
    ServiceClient, ChannelService, ServiceInstance, ServiceRegistry, NamespaceConfig,
    VersionConfig, TagFilter, InstanceMetadata, BackendType, LoadBalanceStrategy, HealthCheckConfig,
};
// KV存储模块
pub use kv::{KvBackend, KvEntry, KvError, KvStore};
pub use types::{ServiceInfo, ServiceType};

// 上下文类型
pub use context::{
    ActorContext, ActorType, DeviceContext, RequestContext, TenantContext, TraceContext,
    // 核心 Context 系统
    Context, ContextExt, ContextError,
};

// gRPC 相关 re-exports
pub use client::*;
pub use health::*;
pub use interceptor::*;
pub use metrics::*;
pub use middleware::*;
pub use retry::*;
// 注意：utils 中的 extract_request_id 已被 middleware 中的版本替代
// 这里不导出 utils，避免命名冲突，如需使用 utils 中的函数请显式导入

// 运行时框架 re-exports
pub use runtime::ServiceRuntime;
pub use runtime::task::{Task, MessageConsumer, MessageConsumerTask, SpawnTask};

// Kafka：挂到 event_bus 下，保持兼容路径 kafka::*
#[cfg(feature = "kafka")]
pub use event_bus::kafka;

// 分布式事件总线
pub use event_bus::{EventEnvelope, TopicEventBus, TopicEventBusError};
#[cfg(feature = "kafka")]
pub use event_bus::{KafkaEventBusConfig, KafkaTopicEventBus};
