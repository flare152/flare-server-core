//! Flare Server Core 错误处理模块
//!
//! 提供统一的错误处理机制，支持国际化、错误代码分类和错误转换
//! 与 flare-core 的错误定义完全适配

pub mod builder;
pub mod code;
pub mod conversions;
pub mod flare_error;
pub mod grpc;
pub mod localized;
pub mod macros;
#[cfg(feature = "proto")]
pub mod proto;

// 重新导出公共类型和函数
pub use builder::ErrorBuilder;
pub use code::{ErrorCategory, ErrorCode};
pub use conversions::AnyhowContext;
pub use flare_error::{FlareError, Result, ServerError};
pub use localized::LocalizedError;
#[cfg(feature = "proto")]
pub use proto::{from_error_detail, ok_error_detail, to_error_detail, to_localized};

// 兼容性导出
pub use flare_error::FlareError as FlareServerError;

/// 基础设施层默认使用的结果类型
pub type InfraResult<T> = anyhow::Result<T>;

/// 将内部错误映射为系统错误，供服务 handler 层统一使用。
#[inline]
pub fn to_system_err(e: impl std::fmt::Display) -> FlareError {
    FlareError::system(format!("Internal error: {e}"))
}

/// 带上下文的系统错误映射，供 MQ 发布、RPC 调用等基础设施边界统一使用。
#[inline]
pub fn to_system_err_with(e: impl std::fmt::Display, context: &str) -> FlareError {
    FlareError::system(format!("{context}: {e}"))
}

/// 上下文取消 / deadline 与统一错误的桥接。
#[inline]
pub fn map_context_error(e: crate::context::ContextError) -> FlareError {
    ErrorBuilder::new(ErrorCode::GeneralError, "context check failed")
        .details(e.to_string())
        .build_error()
}

/// 要求已认证用户 ID（Command / Query / 仓储侧）。
#[inline]
pub fn require_user_id(ctx: &crate::context::Context) -> Result<String> {
    ctx.user_id().map(|s| s.to_string()).ok_or_else(|| {
        ErrorBuilder::new(ErrorCode::AuthenticationRequired, "user_id is required").build_error()
    })
}

/// 将基础设施错误转换为 `FlareError`
pub fn map_infra_error<E, S>(error: E, code: ErrorCode, message: S) -> FlareError
where
    E: std::fmt::Display,
    S: Into<String>,
{
    ErrorBuilder::new(code, message.into())
        .details(error.to_string())
        .build_error()
}

/// `InfraResult` 的辅助扩展，用于快速转换为统一的业务错误类型
pub trait InfraResultExt<T> {
    fn into_flare<S>(self, code: ErrorCode, message: S) -> Result<T>
    where
        S: Into<String>;
}

impl<T> InfraResultExt<T> for InfraResult<T> {
    fn into_flare<S>(self, code: ErrorCode, message: S) -> Result<T>
    where
        S: Into<String>,
    {
        self.map_err(|err| map_infra_error(err, code, message))
    }
}
