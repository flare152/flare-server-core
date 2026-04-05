//! 错误类型转换实现
//!
//! 提供各种错误类型之间的转换

use super::{ErrorCode, FlareError, LocalizedError};
use std::io;

impl From<io::Error> for FlareError {
    fn from(err: io::Error) -> Self {
        FlareError::io(err.to_string())
    }
}

impl From<serde_json::Error> for FlareError {
    fn from(err: serde_json::Error) -> Self {
        FlareError::serialization_error(format!("JSON 序列化错误: {}", err))
    }
}

impl From<tonic::Status> for FlareError {
    fn from(status: tonic::Status) -> Self {
        // 尝试从 status 的 details 中提取错误代码
        let code = status.code();
        let message = status.message();

        // 根据 gRPC 状态码映射到 Flare 错误代码
        let error_code = match code {
            tonic::Code::Unauthenticated => ErrorCode::AuthenticationFailed,
            tonic::Code::PermissionDenied => ErrorCode::PermissionDenied,
            tonic::Code::InvalidArgument => ErrorCode::InvalidParameter,
            tonic::Code::NotFound => ErrorCode::UserNotFound,
            tonic::Code::AlreadyExists => ErrorCode::GeneralError,
            tonic::Code::ResourceExhausted => ErrorCode::ResourceExhausted,
            tonic::Code::Internal => ErrorCode::InternalError,
            tonic::Code::Unavailable => ErrorCode::ServiceUnavailable,
            tonic::Code::DeadlineExceeded => ErrorCode::OperationTimeout,
            _ => ErrorCode::UnknownError,
        };

        FlareError::localized(error_code, message)
    }
}

impl From<LocalizedError> for FlareError {
    fn from(err: LocalizedError) -> Self {
        FlareError::Localized {
            code: err.code,
            reason: err.reason,
            details: err.details,
            params: err.params,
            timestamp: err.timestamp,
        }
    }
}

// ============================================================
// anyhow 错误转换支持
// ============================================================

impl From<anyhow::Error> for FlareError {
    fn from(err: anyhow::Error) -> Self {
        // 将 anyhow::Error 转换为系统错误
        FlareError::system(err.to_string())
    }
}

// ============================================================
// redis 错误转换支持
// ============================================================

impl From<redis::RedisError> for FlareError {
    fn from(err: redis::RedisError) -> Self {
        FlareError::system(format!("Redis error: {}", err))
    }
}

/// 为 anyhow::Result 提供上下文扩展
///
/// 允许使用 `.context()` 和 `.with_context()` 方法将 anyhow 错误转换为 FlareError
pub trait AnyhowContext<T> {
    /// 添加上下文信息
    fn context<C>(self, context: C) -> super::Result<T>
    where
        C: std::fmt::Display + Send + Sync + 'static;

    /// 使用闭包添加上下文信息
    fn with_context<C, F>(self, f: F) -> super::Result<T>
    where
        C: std::fmt::Display + Send + Sync + 'static,
        F: FnOnce() -> C;
}

/// 为所有可以转换为 FlareError 的错误类型提供上下文扩展
///
/// 这允许任何 Result<T, E> 使用 `.context()` 方法，只要 E: Into<FlareError>
impl<T, E> AnyhowContext<T> for Result<T, E>
where
    E: Into<FlareError>,
{
    fn context<C>(self, context: C) -> super::Result<T>
    where
        C: std::fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|err| {
            let flare_err = err.into();
            FlareError::system(format!("{}: {}", context, flare_err))
        })
    }

    fn with_context<C, F>(self, f: F) -> super::Result<T>
    where
        C: std::fmt::Display + Send + Sync + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|err| {
            let context = f();
            let flare_err = err.into();
            FlareError::system(format!("{}: {}", context, flare_err))
        })
    }
}
