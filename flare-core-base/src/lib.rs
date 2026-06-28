//! Shared server primitives for Flare services.
//!
//! `flare-core-base` is the dependency-light foundation used by the rest of
//! the server-core crates. It provides context propagation, typed errors,
//! layered configuration, service metadata, IDs, and small utility helpers.
//!
//! # Main Modules
//!
//! - [`context`] carries tenant, actor, trace, request, device, and audit
//!   metadata through async service boundaries.
//! - [`error`] provides typed error codes, categories, builders, and localized
//!   error payloads.
//! - [`config`] provides layered server, mesh, registry, service, and storage
//!   configuration models.
//! - [`i18n`] provides built-in English and Chinese translation tables for
//!   structured errors.
//! - [`id`] provides Snowflake-style ID generation.

pub mod config;
pub mod context;
pub mod error;
pub mod i18n;
pub mod id;
pub mod types;
pub mod utils;

// Re-exports - Context
pub use context::{
    ActorContext, ActorType, AuditContext, Context, ContextError, ContextExt, Ctx, ExtendedContext,
    TaskControl, TypeMap,
};

// Re-exports - Error
pub use error::{
    ErrorBuilder, ErrorCategory, ErrorCode, FlareError, FlareServerError, LocalizedError, Result,
    ServerError,
};

// Re-exports - Config
pub use config::{
    Config, LayeredConfig, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig,
};

// Re-exports - I18n
pub use i18n::{I18n, default_en_us_translations, default_zh_cn_translations};

// Re-exports - Types
pub use types::{ServiceInfo, ServiceType};

// Re-exports - ID
pub use id::{SnowflakeError, SnowflakeGenerator};

/// Common imports for service code.
pub mod prelude {
    pub use crate::config::Config;
    pub use crate::context::{Context, ContextExt, Ctx};
    pub use crate::error::{ErrorBuilder, FlareError, Result};
}
