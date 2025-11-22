//! Proto/GPRC 错误转换辅助
//!
//! 用于在内部 `FlareError`/`LocalizedError` 与 `flare-proto` 定义的
//! `RpcStatus`/`ErrorCode` 之间相互转换，从而支持跨服务、跨语言的错误传递。

use std::collections::HashMap;
use std::convert::TryFrom;

use chrono::Utc;
use flare_proto::common::{ErrorCode as ProtoErrorCode, ErrorDetail, RpcStatus};

use super::{ErrorCode, FlareError, LocalizedError};

/// metadata 中用于存放原始 Flare 错误码的键
const DETAIL_KEY: &str = "detail";
const FLARE_CODE_KEY: &str = "flare_code";
const PARAM_PREFIX: &str = "param.";
const DETAIL_DOMAIN: &str = "flare.core";
const DETAIL_REASON_METADATA: &str = "metadata";

/// 构造一个代表成功的 `RpcStatus`
#[inline]
pub fn ok_status() -> RpcStatus {
    RpcStatus {
        code: ProtoErrorCode::Ok as i32,
        message: "OK".to_string(),
        details: Vec::new(),
        context: None,
    }
}

/// 将 `FlareError` 转换成 `RpcStatus`，用于跨服务传递
pub fn to_rpc_status(error: &FlareError) -> RpcStatus {
    match error {
        FlareError::Localized {
            code,
            reason,
            details,
            params,
            ..
        } => localized_to_rpc_status(code, reason, details.as_ref(), params.as_ref()),
        FlareError::System(msg) => localized_to_rpc_status(
            &ErrorCode::InternalError,
            msg,
            None,
            None::<&HashMap<String, String>>,
        ),
        FlareError::Io(msg) => localized_to_rpc_status(
            &ErrorCode::NetworkError,
            msg,
            None,
            None::<&HashMap<String, String>>,
        ),
    }
}

/// 将 `LocalizedError` 转为 `RpcStatus`
pub fn localized_to_rpc_status(
    code: &ErrorCode,
    reason: &str,
    details: Option<&String>,
    params: Option<&HashMap<String, String>>,
) -> RpcStatus {
    let mut metadata = HashMap::new();
    if let Some(params) = params {
        for (key, value) in params {
            metadata.insert(format!("{PARAM_PREFIX}{key}"), value.clone());
        }
    }
    metadata.insert(FLARE_CODE_KEY.to_string(), code.as_u32().to_string());

    let detail_message = details.cloned().unwrap_or_default();

    let detail = if metadata.is_empty() && detail_message.is_empty() {
        None
    } else {
        Some(ErrorDetail {
            domain: DETAIL_DOMAIN.to_string(),
            reason: DETAIL_REASON_METADATA.to_string(),
            message: detail_message,
            metadata,
            detail: None,
        })
    };

    RpcStatus {
        code: map_error_code_to_proto(*code) as i32,
        message: reason.to_string(),
        details: detail.into_iter().collect(),
        context: None,
    }
}

/// 从 `RpcStatus` 恢复为 `FlareError`
pub fn from_rpc_status(status: &RpcStatus) -> FlareError {
    let proto_code = ProtoErrorCode::try_from(status.code).unwrap_or(ProtoErrorCode::Internal);

    let mut aggregated_metadata = HashMap::new();
    let mut detail_message = None;
    for detail in &status.details {
        if detail.domain == DETAIL_DOMAIN {
            if detail_message.is_none() && !detail.message.is_empty() {
                detail_message = Some(detail.message.clone());
            }
            for (key, value) in &detail.metadata {
                aggregated_metadata.insert(key.clone(), value.clone());
            }
        }
    }

    let flare_code = aggregated_metadata
        .get(FLARE_CODE_KEY)
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(ErrorCode::from_u32)
        .unwrap_or_else(|| map_proto_code_to_error(proto_code));

    let details = detail_message
        .or_else(|| aggregated_metadata.get(DETAIL_KEY).cloned());

    let params: HashMap<String, String> = aggregated_metadata
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix(PARAM_PREFIX)
                .map(|trimmed| (trimmed.to_string(), value.clone()))
        })
        .collect();

    let params = if params.is_empty() {
        None
    } else {
        Some(params)
    };

    FlareError::Localized {
        code: flare_code,
        reason: status.message.clone(),
        details,
        params,
        timestamp: Utc::now(),
    }
}

/// 将 proto 枚举转换为内部错误码
pub fn map_proto_code_to_error(proto: ProtoErrorCode) -> ErrorCode {
    match proto {
        ProtoErrorCode::InvalidArgument => ErrorCode::InvalidParameter,
        ProtoErrorCode::FailedPrecondition => ErrorCode::OperationFailed,
        ProtoErrorCode::OutOfRange => ErrorCode::InvalidParameter,
        ProtoErrorCode::UnsupportedOperation => ErrorCode::OperationNotSupported,
        ProtoErrorCode::Unauthenticated => ErrorCode::AuthenticationFailed,
        ProtoErrorCode::PermissionDenied => ErrorCode::PermissionDenied,
        ProtoErrorCode::TokenExpired => ErrorCode::TokenExpired,
        ProtoErrorCode::TokenRevoked => ErrorCode::AuthenticationInvalid,
        ProtoErrorCode::NotFound => ErrorCode::UserNotFound,
        ProtoErrorCode::AlreadyExists => ErrorCode::OperationFailed,
        ProtoErrorCode::Conflict => ErrorCode::OperationFailed,
        ProtoErrorCode::ResourceExhausted | ProtoErrorCode::QuotaExceeded => {
            ErrorCode::ResourceExhausted
        }
        ProtoErrorCode::RateLimited => ErrorCode::ResourceExhausted,
        ProtoErrorCode::SlowDown | ProtoErrorCode::BackoffRequired => ErrorCode::ResourceExhausted,
        ProtoErrorCode::InProgress => ErrorCode::OperationFailed,
        ProtoErrorCode::NeedsRetry => ErrorCode::ResourceExhausted,
        ProtoErrorCode::Unavailable => ErrorCode::ServiceUnavailable,
        ProtoErrorCode::Timeout => ErrorCode::OperationTimeout,
        ProtoErrorCode::Ok => ErrorCode::GeneralError,
        ProtoErrorCode::DataLoss => ErrorCode::InternalError,
        ProtoErrorCode::NotImplemented => ErrorCode::OperationNotSupported,
        ProtoErrorCode::Cancelled => ErrorCode::OperationFailed,
        ProtoErrorCode::Unspecified | ProtoErrorCode::Internal => ErrorCode::InternalError,
    }
}

/// 将内部错误码映射为 proto 枚举
pub fn map_error_code_to_proto(code: ErrorCode) -> ProtoErrorCode {
    use ErrorCode::*;

    match code {
        InvalidParameter | MessageFormatError | InvalidCommand => ProtoErrorCode::InvalidArgument,

        AuthenticationFailed
        | AuthenticationExpired
        | AuthenticationInvalid
        | AuthenticationRequired
        | TokenInvalid
        | TokenExpired => ProtoErrorCode::Unauthenticated,

        PermissionDenied => ProtoErrorCode::PermissionDenied,

        UserNotFound | MessageNotFound => ProtoErrorCode::NotFound,

        MessageRateLimitExceeded
        | ConnectionLimitExceeded
        | UserQuotaExceeded
        | ResourceExhausted
        | UserSessionLimitExceeded => ProtoErrorCode::RateLimited,

        NetworkError | NetworkUnreachable | NetworkConnectionLost | ServiceUnavailable => {
            ProtoErrorCode::Unavailable
        }

        ConnectionTimeout | NetworkTimeout | OperationTimeout => ProtoErrorCode::Timeout,

        OperationNotSupported => ProtoErrorCode::UnsupportedOperation,
        OperationFailed => ProtoErrorCode::Conflict,
        // OperationTimeout 已在上面处理，这里移除重复匹配
        UnknownError | GeneralError => ProtoErrorCode::Internal,
        _ => ProtoErrorCode::Internal,
    }
}

/// 辅助：将 `FlareError` 转换为 `LocalizedError`
pub fn to_localized(error: &FlareError) -> LocalizedError {
    match error {
        FlareError::Localized {
            code,
            reason,
            details,
            params,
            timestamp,
        } => LocalizedError {
            code: *code,
            reason: reason.clone(),
            details: details.clone(),
            params: params.clone(),
            timestamp: *timestamp,
        },
        FlareError::System(msg) => LocalizedError::new(ErrorCode::InternalError, msg.clone()),
        FlareError::Io(msg) => LocalizedError::new(ErrorCode::NetworkError, msg.clone()),
    }
}
