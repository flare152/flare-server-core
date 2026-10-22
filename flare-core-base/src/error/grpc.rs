//! gRPC 错误处理（统一：`tonic::Status` + details(ErrorDetail)）。
//!
//! 约定：
//! - 成功：返回正常响应或 `google.protobuf.Empty`
//! - 失败：返回 `tonic::Status`；`grpc-status-details-bin` 是标准的 `google.rpc.Status`：`code`
//!   等于 `grpc-status`，`message` 同 `grpc-message`，结构化错误
//!   `flare_proto::common::ErrorDetail` 作为 `details` 里的一个 `Any`（[`ERROR_DETAIL_TYPE_URL`]）。
//!   grpc-go（grpcurl）、grpc-java 等客户端按规范校验这两处的 code 一致，不一致时只给 Internal。
//! - `reason` 字段作为 i18n key；`message` 作为 fallback

use super::{ErrorCode, FlareError, LocalizedError};
use tonic::{Code, Status};

/// 结构化错误载荷：仅在启用 `proto` feature 时支持解析/构建 details。
#[cfg(feature = "proto")]
pub type ProtoErrorDetail = flare_proto::common::ErrorDetail;

/// `ErrorDetail` 在 `google.rpc.Status.details` 里的 `Any.type_url`。
#[cfg(feature = "proto")]
pub const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/flare.common.v1.ErrorDetail";

/// `google.rpc.Status`（`grpc-status-details-bin` 的标准载荷）。
#[cfg(feature = "proto")]
#[derive(Clone, PartialEq, prost::Message)]
struct RpcStatus {
    #[prost(int32, tag = "1")]
    code: i32,
    #[prost(string, tag = "2")]
    message: String,
    #[prost(message, repeated, tag = "3")]
    details: Vec<RpcAny>,
}

/// `google.protobuf.Any`。
#[cfg(feature = "proto")]
#[derive(Clone, PartialEq, prost::Message)]
struct RpcAny {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes = "vec", tag = "2")]
    value: Vec<u8>,
}

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

/// 从 gRPC `Status.details`（`google.rpc.Status`）取出 `ErrorDetail`；不是本系统编码时为 None。
#[cfg(feature = "proto")]
pub fn decode_error_detail(status: &Status) -> Option<ProtoErrorDetail> {
    use prost::Message as _;
    RpcStatus::decode(status.details())
        .ok()?
        .details
        .into_iter()
        .find(|any| any.type_url == ERROR_DETAIL_TYPE_URL)
        .and_then(|any| ProtoErrorDetail::decode(any.value.as_slice()).ok())
}

/// 判断该 Status 是否包含本系统的结构化错误 detail。
#[cfg(feature = "proto")]
#[inline]
pub fn has_error_detail(status: &Status) -> bool {
    decode_error_detail(status).is_some()
}

/// 构造 `tonic::Status`：`grpc-status` 为 `code`，details 是同一 `code` 的 `google.rpc.Status`，
/// 携带 `detail`。
#[cfg(feature = "proto")]
pub fn build_status(code: Code, detail: ProtoErrorDetail) -> Status {
    use prost::Message as _;
    let msg = if detail.message.is_empty() {
        detail.reason.clone()
    } else {
        detail.message.clone()
    };
    let details = RpcStatus {
        code: code as i32,
        message: msg.clone(),
        details: vec![RpcAny {
            type_url: ERROR_DETAIL_TYPE_URL.to_string(),
            value: detail.encode_to_vec(),
        }],
    }
    .encode_to_vec();
    Status::with_details(code, msg, details.into())
}

/// 换一个 gRPC 状态码重发同一个错误：`grpc-status` 与 details 里的 `code` 一起换，`ErrorDetail`
/// 与说明不变。没有本系统 details 的 Status 只换状态码。
#[cfg(feature = "proto")]
pub fn with_code(status: Status, code: Code) -> Status {
    match decode_error_detail(&status) {
        Some(detail) => build_status(code, detail),
        None => Status::new(code, status.message()),
    }
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
    #[cfg(feature = "proto")]
    fn from(err: FlareError) -> Self {
        let (code, track) = match &err {
            FlareError::Localized { code, .. } => (map_error_code_to_grpc(*code), ""),
            FlareError::System(_) => (Code::Internal, ""),
            FlareError::Io(_) => (Code::Unavailable, ""),
        };
        build_status(code, to_error_detail(&err, track))
    }

    #[cfg(not(feature = "proto"))]
    fn from(err: FlareError) -> Self {
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

#[cfg(all(test, feature = "proto"))]
mod tests {
    use super::*;
    use prost::Message as _;

    fn detail() -> ProtoErrorDetail {
        ProtoErrorDetail {
            code: ErrorCode::InvalidParameter as i32,
            reason: "SYNC_SEND_UNSUPPORTED".into(),
            message: "sync persisted send is not supported".into(),
            track: String::new(),
        }
    }

    /// What grpc-go and grpc-java check: the details are a `google.rpc.Status` with the same code
    /// and message as the status itself.
    #[test]
    fn details_are_a_google_rpc_status_matching_the_status() {
        let status = build_status(Code::InvalidArgument, detail());
        let rpc = RpcStatus::decode(status.details()).unwrap();
        assert_eq!(rpc.code, Code::InvalidArgument as i32);
        assert_eq!(rpc.code, status.code() as i32);
        assert_eq!(rpc.message, status.message());
        assert_eq!(rpc.details.len(), 1);
        assert_eq!(rpc.details[0].type_url, ERROR_DETAIL_TYPE_URL);
        assert_eq!(decode_error_detail(&status), Some(detail()));
        assert!(has_error_detail(&status));
    }

    #[test]
    fn every_flare_error_carries_consistent_details() {
        for err in [
            FlareError::localized(ErrorCode::OperationNotSupported, "OPERATION_NOT_SUPPORTED"),
            FlareError::localized(ErrorCode::PermissionDenied, "FORBIDDEN"),
            FlareError::system("boom"),
            FlareError::io("down"),
        ] {
            let status = Status::from(err);
            let rpc = RpcStatus::decode(status.details()).unwrap();
            assert_eq!(rpc.code, status.code() as i32, "{status:?}");
            assert!(decode_error_detail(&status).is_some());
        }
    }

    #[test]
    fn a_new_code_moves_both_the_status_and_its_details() {
        let status = with_code(
            build_status(Code::Unknown, detail()),
            Code::FailedPrecondition,
        );
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert_eq!(
            RpcStatus::decode(status.details()).unwrap().code,
            Code::FailedPrecondition as i32
        );
        assert_eq!(decode_error_detail(&status), Some(detail()));
        let plain = with_code(Status::unknown("plain"), Code::Unavailable);
        assert_eq!(
            (plain.code(), plain.message()),
            (Code::Unavailable, "plain")
        );
        assert!(plain.details().is_empty());
    }

    #[test]
    fn foreign_or_bare_details_carry_no_error_detail() {
        assert_eq!(decode_error_detail(&Status::internal("plain")), None);
        // The bare `ErrorDetail` bytes of the old encoding are no `google.rpc.Status` with an
        // `ErrorDetail` inside.
        let bare = Status::with_details(Code::Unknown, "x", detail().encode_to_vec().into());
        assert_eq!(decode_error_detail(&bare), None);
    }
}
