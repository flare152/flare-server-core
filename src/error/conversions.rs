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
