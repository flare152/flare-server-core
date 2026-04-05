//! gRPC 错误处理（统一：`tonic::Status` + details(ErrorDetail)）。
//!
//! 约定：
//! - 成功：返回正常响应或 `google.protobuf.Empty`
//! - 失败：返回 `tonic::Status`，并在 `details` 中携带 `flare_proto::common::ErrorDetail`
//! - `reason` 字段作为 i18n key；`message` 作为 fallback

use super::{ErrorCode, FlareError, LocalizedError};
use tonic::{Code, Status};

/// 结构化错误载荷：仅在启用 `proto` feature 时支持解析/构建 details。
#[cfg(feature = "proto")]
pub type ProtoErrorDetail = flare_proto::common::ErrorDetail;

/// 将 `FlareError` 转换为结构化 `ErrorDetail`（用于 Status details）。
#[cfg(feature = "proto")]
#[inline]
pub fn to_error_detail(err: &FlareError, track: impl Into<String>) -> ProtoErrorDetail {
    crate::error::to_error_detail(err, track)
}

/// 成功用的 ErrorDetail（一般不用返回；仅供非 gRPC 场景）。
#[cfg(feature = "proto")]
#[inline]
pub fn ok_error_detail(track: impl Into<String>) -> ProtoErrorDetail {
    crate::error::ok_error_detail(track)
}

/// 将 `ErrorDetail` 编码为 gRPC `Status.details`（prost）。
#[cfg(feature = "proto")]
#[inline]
pub fn encode_error_detail(detail: &ProtoErrorDetail) -> bytes::Bytes {
    use prost::Message as _;
    bytes::Bytes::from(detail.encode_to_vec())
}

/// 从 gRPC `Status.details` 解析出 `ErrorDetail`（若不是本系统编码则返回 None）。
#[cfg(feature = "proto")]
pub fn decode_error_detail(status: &Status) -> Option<ProtoErrorDetail> {
    use prost::Message as _;
    ProtoErrorDetail::decode(status.details()).ok()
}

/// 判断该 Status 是否包含本系统的结构化错误 detail。
#[cfg(feature = "proto")]
#[inline]
pub fn has_error_detail(status: &Status) -> bool {
    decode_error_detail(status).is_some()
}

/// 快速构造 `tonic::Status`（携带 details(ErrorDetail)）。
#[cfg(feature = "proto")]
pub fn build_status(code: Code, detail: ProtoErrorDetail) -> Status {
    let msg = if detail.message.is_empty() {
        detail.reason.clone()
    } else {
        detail.message.clone()
    };
    Status::with_details(code, msg, encode_error_detail(&detail))
}

/// 内部错误码 → gRPC Code 映射（与 ErrorCode 语义对齐）。
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

        // 同步：前置条件不满足
        ErrorCode::SyncCursorRegression => Code::FailedPrecondition,

        _ => Code::Unknown,
    }
}

impl From<FlareError> for Status {
    fn from(err: FlareError) -> Self {
        #[cfg(feature = "proto")]
        {
            let (code, track) = match &err {
                FlareError::Localized { code, .. } => (map_error_code_to_grpc(*code), ""),
                FlareError::System(_) => (Code::Internal, ""),
                FlareError::Io(_) => (Code::Unavailable, ""),
            };
            return build_status(code, to_error_detail(&err, track));
        }

        #[cfg(not(feature = "proto"))]
        match err {
            FlareError::Localized {
                code,
                reason,
                details,
                ..
            } => {
                let msg = details.unwrap_or_else(|| reason.clone());
                Status::new(map_error_code_to_grpc(code), msg)
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

/// 业务侧 `Result<T, FlareError>` → gRPC `Result<T, tonic::Status>`。
pub trait IntoGrpc<T> {
    fn into_grpc(self) -> std::result::Result<T, Status>;
}

impl<T> IntoGrpc<T> for super::Result<T> {
    fn into_grpc(self) -> std::result::Result<T, Status> {
        self.map_err(Status::from)
    }
}

/// 判断是否为成功（gRPC Code == OK）。
#[inline]
pub fn is_ok(status: &Status) -> bool {
    status.code() == Code::Ok
}

/// 判断是否为错误（gRPC Code != OK）。
#[inline]
pub fn is_err(status: &Status) -> bool {
    status.code() != Code::Ok
}

/// 宏：构造带结构化 details 的 Status（需启用 `proto` feature）。
#[macro_export]
macro_rules! grpc_err {
    ($code:expr, $biz_code:expr, $reason:expr, $message:expr, $track:expr) => {{
        #[cfg(feature = "proto")]
        {
            let detail = flare_proto::common::ErrorDetail {
                code: $biz_code,
                reason: $reason.to_string(),
                message: $message.to_string(),
                track: $track.to_string(),
            };
            $crate::error::grpc::build_status($code, detail)
        }
        #[cfg(not(feature = "proto"))]
        {
            tonic::Status::new($code, $message)
        }
    }};
}

#[macro_export]
macro_rules! grpc_invalid_argument {
    ($biz_code:expr, $reason:expr, $message:expr) => {
        $crate::grpc_err!(
            tonic::Code::InvalidArgument,
            $biz_code,
            $reason,
            $message,
            ""
        )
    };
}

#[macro_export]
macro_rules! grpc_unauthenticated {
    ($biz_code:expr, $reason:expr, $message:expr) => {
        $crate::grpc_err!(
            tonic::Code::Unauthenticated,
            $biz_code,
            $reason,
            $message,
            ""
        )
    };
}

#[macro_export]
macro_rules! grpc_permission_denied {
    ($biz_code:expr, $reason:expr, $message:expr) => {
        $crate::grpc_err!(
            tonic::Code::PermissionDenied,
            $biz_code,
            $reason,
            $message,
            ""
        )
    };
}

#[macro_export]
macro_rules! grpc_not_found {
    ($biz_code:expr, $reason:expr, $message:expr) => {
        $crate::grpc_err!(tonic::Code::NotFound, $biz_code, $reason, $message, "")
    };
}

#[macro_export]
macro_rules! grpc_internal {
    ($biz_code:expr, $reason:expr, $message:expr) => {
        $crate::grpc_err!(tonic::Code::Internal, $biz_code, $reason, $message, "")
    };
}
