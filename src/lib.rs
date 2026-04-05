//! Flare Server Core - 统一入口
//!
//! 重新导出所有子 crate 的公共 API

// 重新导出所有子 crate
pub use flare_core_base;
pub use flare_core_infra;
pub use flare_core_messaging;
pub use flare_core_runtime;
pub use flare_core_transport;

// 重新导出常用模块
pub use flare_core_base::config;
pub use flare_core_base::context;
pub use flare_core_base::error;
pub use flare_core_base::types;

// utils 模块 (包含 gRPC 工具函数)
#[cfg(not(feature = "grpc"))]
pub use flare_core_base::utils;

#[cfg(feature = "grpc")]
pub mod utils {
    // 重新导出 base utils
    pub use flare_core_base::utils::*;
    // 添加 gRPC 相关工具函数
    pub use flare_core_transport::grpc::utils::{
        extract_ctx_from_request_opt, require_ctx_from_request,
    };
}

// 重新导出错误类型和宏
pub use flare_core_base::error::{ErrorCode, FlareError};

// 重新导出错误宏
pub use flare_core_base::{flare_err, flare_err_details};

// 重新导出上下文类型
pub use flare_core_base::context::Context;

// 重新导出配置类型
pub use flare_core_base::config::{
    Config, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig,
};

// gRPC utils (需要 grpc feature)
#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc::utils::{
    extract_ctx_from_request_opt, require_ctx_from_request,
};

// gRPC client (需要 grpc feature)
#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc::client::{
    set_context_metadata, request_with_context,
    ClientContextConfig, ClientContextInterceptor,
    context_interceptor_with_tenant, context_interceptor_with_tenant_and_user,
    default_context_interceptor,
};

// gRPC middleware (需要 grpc feature)
#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc::middleware::{
    ContextLayer, ContextService,
    extract_actor_id, extract_context, extract_request_id,
    extract_tenant_id, extract_user_id, get_context,
    require_actor_id, require_request_id, require_tenant_id, require_user_id,
};

// 传输层 (需要对应 feature)
#[cfg(feature = "http")]
pub use flare_core_transport::http;

#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc;

#[cfg(feature = "discovery")]
pub use flare_core_transport::discovery;

// 服务发现类型 (需要 discovery feature)
#[cfg(feature = "discovery")]
pub use flare_core_transport::discovery::{DiscoveryFactory, LoadBalanceStrategy, ServiceDiscover};

// 消息层
pub use flare_core_messaging::eventbus;
pub use flare_core_messaging::mq;

// Kafka 支持 (需要 kafka feature)
#[cfg(feature = "kafka")]
pub use flare_core_messaging::mq::kafka;

// 重新导出 eventbus 的常用类型
pub use flare_core_messaging::eventbus::{
    DEFAULT_TOPIC_BROADCAST_CAPACITY, EVENT_ENVELOPE_CONTENT_TYPE, EventBus, EventEnvelope,
    EventSubscriber, HEADER_CONTENT_TYPE, InMemoryEventBus, InMemoryTopicEventBus, MqEventHandler,
    MqTopicEventBus, TopicBroadcast, TopicEventBus, register_event_handler, run_event_consumer,
};

// 基础设施
pub use flare_core_infra::auth;
pub use flare_core_infra::kv;
pub use flare_core_infra::telemetry;

// 认证类型
pub use flare_core_infra::auth::{TokenClaims, TokenService};

// KV 存储类型
pub use flare_core_infra::kv::{KvBackend, KvStore};

// 重新导出 telemetry 常用类型
pub use flare_core_infra::telemetry::{LoggingSubscriberOptions, init_fmt_subscriber};

// 运行时
pub use flare_core_runtime::ServiceRuntime;
pub use flare_core_runtime::config as runtime_config;
pub use flare_core_runtime::task;

// 运行时模块
pub use flare_core_runtime as runtime;

// 客户端 (需要 discovery feature)
#[cfg(feature = "discovery")]
pub use flare_core_transport::discovery::ServiceClient;

// 中间件模块
#[cfg(all(feature = "http", not(feature = "grpc")))]
pub use flare_core_transport::http::middleware;

#[cfg(all(feature = "grpc", not(feature = "http")))]
pub mod middleware {
    pub use flare_core_transport::grpc::middleware::{
        ContextLayer, ContextService,
        extract_actor_id, extract_context, extract_request_id,
        extract_tenant_id, extract_user_id, get_context,
        require_actor_id, require_request_id, require_tenant_id, require_user_id,
    };
}

#[cfg(all(feature = "http", feature = "grpc"))]
pub mod middleware {
    // HTTP middleware
    pub use flare_core_transport::http::middleware::*;
    // gRPC middleware
    pub use flare_core_transport::grpc::middleware::{
        ContextLayer, ContextService,
        extract_actor_id, extract_context, extract_request_id,
        extract_tenant_id, extract_user_id, get_context,
        require_actor_id, require_request_id, require_tenant_id, require_user_id,
    };
}

// gRPC client 模块 (需要 grpc feature)
#[cfg(feature = "grpc")]
pub mod client {
    pub use flare_core_transport::grpc::client::{
        set_context_metadata, request_with_context,
        ClientContextConfig, ClientContextInterceptor,
        context_interceptor_with_tenant, context_interceptor_with_tenant_and_user,
        default_context_interceptor,
    };
}

// 客户端模块 (需要 discovery feature)
#[cfg(feature = "discovery")]
pub mod discovery_client {
    pub use flare_core_transport::discovery::ServiceClient;
}
