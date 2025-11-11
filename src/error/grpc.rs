//! gRPC 错误处理
//!
//! 提供 gRPC 错误与 FlareError 之间的转换

use super::{ErrorCode, FlareError, LocalizedError};
use thiserror::Error;
use tonic::{Code, Status};

/// gRPC 错误类型（用于兼容性）
#[derive(Error, Debug)]
pub enum GrpcError {
    #[error("Unauthenticated: {0}")]
    Unauthenticated(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Resource exhausted: {0}")]
    ResourceExhausted(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Unavailable: {0}")]
    Unavailable(String),

    #[error("Deadline exceeded: {0}")]
    DeadlineExceeded(String),
}

/// gRPC 结果类型
pub type GrpcResult<T> = std::result::Result<T, GrpcError>;

/// gRPC 错误扩展 trait
pub trait GrpcErrorExt<T> {
    fn to_status(self) -> std::result::Result<T, Status>;
}

impl<T> GrpcErrorExt<T> for GrpcResult<T> {
    fn to_status(self) -> std::result::Result<T, Status> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => Err(err.into()),
        }
    }
}

impl From<GrpcError> for Status {
    fn from(err: GrpcError) -> Self {
        match err {
            GrpcError::Unauthenticated(msg) => Status::unauthenticated(msg),
            GrpcError::PermissionDenied(msg) => Status::permission_denied(msg),
            GrpcError::InvalidArgument(msg) => Status::invalid_argument(msg),
            GrpcError::NotFound(msg) => Status::not_found(msg),
            GrpcError::AlreadyExists(msg) => Status::already_exists(msg),
            GrpcError::ResourceExhausted(msg) => Status::resource_exhausted(msg),
            GrpcError::Internal(msg) => Status::internal(msg),
            GrpcError::Unavailable(msg) => Status::unavailable(msg),
            GrpcError::DeadlineExceeded(msg) => Status::deadline_exceeded(msg),
        }
    }
}

impl From<FlareError> for Status {
    fn from(err: FlareError) -> Self {
        match err {
            FlareError::Localized {
                code,
                reason,
                details,
                ..
            } => {
                let grpc_code = map_error_code_to_grpc(code);
                let mut status = Status::new(grpc_code, reason);

                // 将错误信息添加到 status 的 metadata 中
                if let Some(details) = details {
                    if let Ok(value) = details.parse() {
                        status.metadata_mut().insert("error-details", value);
                    }
                }

                // 添加错误代码到 metadata
                if let Ok(value) = code.as_u32().to_string().parse() {
                    status.metadata_mut().insert("error-code", value);
                }

                status
            }
            FlareError::System(msg) => Status::internal(msg),
            FlareError::Io(msg) => Status::unavailable(msg),
        }
    }
}

impl From<LocalizedError> for Status {
    fn from(err: LocalizedError) -> Self {
        let flare_err: FlareError = err.into();
        flare_err.into()
    }
}

/// 将 Flare 错误代码映射到 gRPC 状态码
fn map_error_code_to_grpc(code: ErrorCode) -> Code {
    match code {
        // 认证相关
        ErrorCode::AuthenticationFailed
        | ErrorCode::AuthenticationExpired
        | ErrorCode::AuthenticationInvalid
        | ErrorCode::AuthenticationRequired
        | ErrorCode::TokenInvalid
        | ErrorCode::TokenExpired => Code::Unauthenticated,

        // 权限相关
        ErrorCode::PermissionDenied => Code::PermissionDenied,

        // 参数相关
        ErrorCode::InvalidParameter | ErrorCode::MessageFormatError | ErrorCode::InvalidCommand => {
            Code::InvalidArgument
        }

        // 未找到
        ErrorCode::UserNotFound | ErrorCode::MessageNotFound => Code::NotFound,

        // 资源耗尽
        ErrorCode::ResourceExhausted
        | ErrorCode::ConnectionLimitExceeded
        | ErrorCode::UserQuotaExceeded
        | ErrorCode::MessageRateLimitExceeded => Code::ResourceExhausted,

        // 服务不可用
        ErrorCode::ServiceUnavailable
        | ErrorCode::NetworkError
        | ErrorCode::NetworkUnreachable
        | ErrorCode::NetworkConnectionLost => Code::Unavailable,

        // 超时
        ErrorCode::ConnectionTimeout | ErrorCode::NetworkTimeout | ErrorCode::OperationTimeout => {
            Code::DeadlineExceeded
        }

        // 内部错误
        ErrorCode::InternalError
        | ErrorCode::DatabaseError
        | ErrorCode::ConfigurationError
        | ErrorCode::SerializationError
        | ErrorCode::DeserializationError
        | ErrorCode::EncodingError => Code::Internal,

        // 其他错误
        _ => Code::Unknown,
    }
}
