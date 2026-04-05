//! Proto 错误转换辅助（仅在启用 `proto` feature 时生效）。
//!
//! 当前协议约定：业务响应不再显式携带 `status` 字段；错误通过 gRPC `tonic::Status` 的
//! `details` 承载 `flare_proto::common::ErrorDetail`（见 `crate::error::grpc`）。
//!
//! 本模块仅提供：内部错误类型与 `flare-proto` 的 `ErrorDetail` 之间的映射工具，
//! 便于业务在非 gRPC 场景（如 MQ/WS）仍可携带结构化错误。

#[cfg(feature = "proto")]
use super::{ErrorCode, FlareError, LocalizedError};

#[cfg(feature = "proto")]
use flare_proto::common::ErrorDetail as ProtoErrorDetail;

/// 成功：用于非 gRPC 的结构化回包（gRPC 成功直接返回正常响应或 Empty）。
#[cfg(feature = "proto")]
#[inline]
pub fn ok_error_detail(track: impl Into<String>) -> ProtoErrorDetail {
    ProtoErrorDetail {
        code: 0,
        reason: "OK".to_string(),
        message: String::new(),
        track: track.into(),
    }
}

/// 将内部 FlareError 映射为 `flare-proto` 的 `ErrorDetail`（供 MQ/WS 等携带）。
#[cfg(feature = "proto")]
pub fn to_error_detail(err: &FlareError, track: impl Into<String>) -> ProtoErrorDetail {
    let track = track.into();
    match err {
        FlareError::Localized {
            code,
            reason,
            details,
            ..
        } => ProtoErrorDetail {
            code: code.as_u32() as i32,
            reason: reason.clone(),
            message: details.clone().unwrap_or_default(),
            track,
        },
        FlareError::System(msg) => ProtoErrorDetail {
            code: ErrorCode::InternalError.as_u32() as i32,
            reason: "internal".to_string(),
            message: msg.clone(),
            track,
        },
        FlareError::Io(msg) => ProtoErrorDetail {
            code: ErrorCode::NetworkError.as_u32() as i32,
            reason: "network".to_string(),
            message: msg.clone(),
            track,
        },
    }
}

/// 从 `ErrorDetail` 还原为内部 FlareError（用于非 gRPC 场景）。
#[cfg(feature = "proto")]
pub fn from_error_detail(detail: &ProtoErrorDetail) -> FlareError {
    let code = ErrorCode::from_u32(detail.code.max(0) as u32).unwrap_or(ErrorCode::GeneralError);
    FlareError::Localized {
        code,
        reason: detail.reason.clone(),
        details: if detail.message.is_empty() {
            None
        } else {
            Some(detail.message.clone())
        },
        params: None,
        timestamp: chrono::Utc::now(),
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
