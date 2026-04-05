//! Flare Core Base - 基础类型和工具
//!
//! 提供上下文传播、错误处理、配置管理、国际化等核心能力
//!
//! # 模块
//!
//! - `context`: 上下文传播系统,支持 TraceID、UserID、TenantID 等追踪信息
//! - `error`: 统一错误处理,支持错误码、分类、国际化消息
//! - `config`: 配置管理,支持服务、Mesh、存储等配置
//! - `i18n`: 国际化支持,多语言错误消息
//! - `types`: 公共类型定义

pub mod config;
pub mod context;
pub mod error;
pub mod i18n;
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
pub use config::{Config, MeshConfig, RegistryConfig, ServerConfig, ServiceConfig, StorageConfig};

// Re-exports - I18n
pub use i18n::{I18n, default_en_us_translations, default_zh_cn_translations};

// Re-exports - Types
pub use types::{ServiceInfo, ServiceType};

/// Prelude module - 常用类型导入
pub mod prelude {
    pub use crate::config::Config;
    pub use crate::context::{Context, ContextExt, Ctx};
    pub use crate::error::{ErrorBuilder, FlareError, Result};
}
