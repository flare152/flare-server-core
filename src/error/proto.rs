//! Proto/GPRC 错误转换辅助
//!
//! 用于在内部 `FlareError`/`LocalizedError` 与 `flare-proto` 定义的
//! `RpcStatus`/`ErrorCode` 之间相互转换，从而支持跨服务、跨语言的错误传递。

use std::collections::HashMap;
use std::convert::TryFrom;

use chrono::Utc;
use flare_proto::common::{ErrorCode as ProtoErrorCode, RpcStatus};

use super::{ErrorCode, FlareError, LocalizedError};

/// metadata 中用于存放原始 Flare 错误码的键
const DETAIL_KEY: &str = "detail";
const FLARE_CODE_KEY: &str = "flare_code";
const PARAM_PREFIX: &str = "param.";

/// 构造一个代表成功的 `RpcStatus`
#[inline]
pub fn ok_status() -> RpcStatus {
    RpcStatus {
        code: ProtoErrorCode::Ok as i32,
        message: "OK".to_string(),
        details: HashMap::new(),
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
    let mut extra = HashMap::new();
    if let Some(details) = details {
        extra.insert(DETAIL_KEY.to_string(), details.clone());
    }
    if let Some(params) = params {
        for (key, value) in params {
            extra.insert(format!("{PARAM_PREFIX}{key}"), value.clone());
        }
    }
    extra.insert(FLARE_CODE_KEY.to_string(), code.as_u32().to_string());

    RpcStatus {
        code: map_error_code_to_proto(*code) as i32,
        message: reason.to_string(),
        details: extra,
    }
}

/// 从 `RpcStatus` 恢复为 `FlareError`
pub fn from_rpc_status(status: &RpcStatus) -> FlareError {
    let proto_code = ProtoErrorCode::try_from(status.code).unwrap_or(ProtoErrorCode::Internal);

    let flare_code = status
        .details
        .get(FLARE_CODE_KEY)
        .and_then(|value| value.parse::<u32>().ok())
        .and_then(ErrorCode::from_u32)
        .unwrap_or_else(|| map_proto_code_to_error(proto_code));

    let details = status.details.get(DETAIL_KEY).cloned();

    let params: HashMap<String, String> = status
        .details
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
        ProtoErrorCode::Unauthenticated => ErrorCode::AuthenticationFailed,
        ProtoErrorCode::PermissionDenied => ErrorCode::PermissionDenied,
        ProtoErrorCode::NotFound => ErrorCode::UserNotFound,
        ProtoErrorCode::Conflict => ErrorCode::OperationFailed,
        ProtoErrorCode::RateLimited => ErrorCode::ResourceExhausted,
        ProtoErrorCode::Unavailable => ErrorCode::ServiceUnavailable,
        ProtoErrorCode::Timeout => ErrorCode::OperationTimeout,
        ProtoErrorCode::Ok => ErrorCode::GeneralError,
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
