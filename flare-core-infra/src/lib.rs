//! Flare Core Infrastructure - 基础设施层
//!
//! 提供 KV 存储、认证、可观测性等基础设施能力

pub mod auth;
pub mod kv;
pub mod metrics;
pub mod telemetry;

// Re-exports - KV
pub use kv::{KvBackend, KvEntry, KvError, KvStore};

// Re-exports - Auth
pub use auth::{TokenClaims, TokenService};

// Re-exports - Telemetry
pub use telemetry::{LoggingSubscriberOptions, init_fmt_subscriber};
