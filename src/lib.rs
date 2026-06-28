//! Aggregated server-side infrastructure toolkit for Flare services.
//!
//! `flare-server-core` re-exports the lower-level server infrastructure crates
//! under one dependency for application services that want a compact import
//! surface. It covers runtime lifecycle, context propagation, typed errors,
//! HTTP/gRPC helpers, service discovery, event buses, message queues,
//! authentication, KV abstractions, and telemetry setup.
//!
//! # Crate Layout
//!
//! - [`flare_core_base`] provides context, errors, configuration, IDs, and
//!   shared service types.
//! - [`flare_core_runtime`] provides service lifecycle, task orchestration,
//!   health checks, shutdown signals, and state tracking.
//! - [`flare_core_infra`] provides KV, authentication, metrics, and telemetry
//!   helpers.
//! - [`flare_core_transport`] provides HTTP, gRPC, service discovery, and
//!   transport middleware.
//! - [`flare_core_messaging`] provides event-bus, topic-bus, NATS, and Kafka
//!   primitives.
//!
//! # Feature Flags
//!
//! Enable only the integration surfaces you need:
//!
//! - `http`: HTTP response and middleware helpers.
//! - `grpc`: tonic context, client, interceptor, and middleware helpers.
//! - `discovery`: service discovery and service clients.
//! - `nats` / `kafka`: MQ integrations.
//! - `kv`, `auth`, `telemetry`: infrastructure adapters.
//! - `proto`: optional bridge to `flare-proto` structured payloads.
//! - `full`: all public capabilities.

// Re-export all lower-level crates.
pub use flare_core_base;
pub use flare_core_infra;
pub use flare_core_messaging;
pub use flare_core_runtime;
pub use flare_core_transport;

// Common module re-exports.
pub use flare_core_base::config;
pub use flare_core_base::context;
pub use flare_core_base::error;
pub use flare_core_base::types;

// Utility module. With `grpc`, this also includes gRPC context helpers.
#[cfg(not(feature = "grpc"))]
pub use flare_core_base::utils;

#[cfg(feature = "grpc")]
pub mod utils {
    // Base utilities.
    pub use flare_core_base::utils::*;
    // gRPC utilities.
    pub use flare_core_transport::grpc::utils::{
        extract_ctx_from_request_opt, require_ctx_from_request,
    };
}

// Error types and macros.
pub use flare_core_base::error::{ErrorCode, FlareError};

// Error macros.
pub use flare_core_base::{flare_err, flare_err_details};

// Context type.
pub use flare_core_base::context::Context;

// Configuration types.
pub use flare_core_base::config::{
    Config, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig,
};

// gRPC utilities.
#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc::utils::{
    extract_ctx_from_request_opt, require_ctx_from_request,
};

// gRPC client helpers.
#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc::client::{
    ClientContextConfig, ClientContextInterceptor, context_interceptor_with_tenant,
    context_interceptor_with_tenant_and_user, default_context_interceptor, request_with_context,
    set_context_metadata,
};

// gRPC middleware helpers.
#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc::middleware::{
    ContextLayer, ContextService, extract_actor_id, extract_context, extract_request_id,
    extract_tenant_id, extract_user_id, get_context, require_actor_id, require_request_id,
    require_tenant_id, require_user_id,
};

// Transport modules.
#[cfg(feature = "http")]
pub use flare_core_transport::http;

#[cfg(feature = "grpc")]
pub use flare_core_transport::grpc;

#[cfg(feature = "discovery")]
pub use flare_core_transport::discovery;

// Service discovery types.
#[cfg(feature = "discovery")]
pub use flare_core_transport::discovery::{DiscoveryFactory, LoadBalanceStrategy, ServiceDiscover};

// Messaging layer.
pub use flare_core_messaging::eventbus;
pub use flare_core_messaging::mq;

// NATS JetStream support.
#[cfg(feature = "nats")]
pub use flare_core_messaging::mq::nats;

// Common event-bus types.
pub use flare_core_messaging::eventbus::{
    DEFAULT_TOPIC_BROADCAST_CAPACITY, EVENT_ENVELOPE_CONTENT_TYPE, EventBus, EventEnvelope,
    EventSubscriber, HEADER_CONTENT_TYPE, InMemoryEventBus, InMemoryTopicEventBus, MqEventHandler,
    MqTopicEventBus, TopicBroadcast, TopicEventBus, register_event_handler, run_event_consumer,
};

// Infrastructure modules.
pub use flare_core_infra::auth;
pub use flare_core_infra::kv;
pub use flare_core_infra::telemetry;

// Authentication types.
pub use flare_core_infra::auth::{
    AuthError, AuthenticatedPrincipal, CompositeTokenValidator, TokenClaims, TokenService,
    TokenValidationRequest, TokenValidator, TrustedIssuer,
};

// KV storage types.
pub use flare_core_infra::kv::{KvBackend, KvStore};

// Common telemetry types.
pub use flare_core_infra::telemetry::{
    LoggingSubscriberOptions, OtlpTracingOptions, init_fmt_subscriber, init_tracing_subscriber,
};

// Runtime types.
pub use flare_core_runtime::ServiceRuntime;
pub use flare_core_runtime::config as runtime_config;
pub use flare_core_runtime::task;

// Runtime module.
pub use flare_core_runtime as runtime;

// Service discovery client.
#[cfg(feature = "discovery")]
pub use flare_core_transport::discovery::ServiceClient;

// Middleware module.
#[cfg(all(feature = "http", not(feature = "grpc")))]
pub use flare_core_transport::http::middleware;

#[cfg(all(feature = "grpc", not(feature = "http")))]
pub mod middleware {
    pub use flare_core_transport::grpc::middleware::{
        ContextLayer, ContextService, extract_actor_id, extract_context, extract_request_id,
        extract_tenant_id, extract_user_id, get_context, require_actor_id, require_request_id,
        require_tenant_id, require_user_id,
    };
}

#[cfg(all(feature = "http", feature = "grpc"))]
pub mod middleware {
    // HTTP middleware.
    pub use flare_core_transport::http::middleware::*;
    // gRPC middleware.
    pub use flare_core_transport::grpc::middleware::{
        ContextLayer, ContextService, extract_actor_id, extract_context, extract_request_id,
        extract_tenant_id, extract_user_id, get_context, require_actor_id, require_request_id,
        require_tenant_id, require_user_id,
    };
}

// gRPC client module.
#[cfg(feature = "grpc")]
pub mod client {
    pub use flare_core_transport::grpc::client::{
        ClientContextConfig, ClientContextInterceptor, context_interceptor_with_tenant,
        context_interceptor_with_tenant_and_user, default_context_interceptor,
        request_with_context, set_context_metadata,
    };
}

// Discovery client module.
#[cfg(feature = "discovery")]
pub mod discovery_client {
    pub use flare_core_transport::discovery::ServiceClient;
}
