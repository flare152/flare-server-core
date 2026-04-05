//! HTTP 响应模型
//!
//! 提供统一的 HTTP API 响应格式,支持与 FlareError 的转换

use serde::{Deserialize, Serialize};

#[cfg(feature = "proto")]
use flare_proto::common::ErrorDetail as ProtoErrorDetail;

use flare_core_base::error::{ErrorCode, FlareError};

/// 统一 API 响应
///
/// 使用 code 标记是否成功:
/// - code = 200: 成功
/// - code > 0: 错误
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    /// 状态码 (0=成功, >0=错误)
    pub code: i32,
    /// 响应数据
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// 错误原因 (仅错误时有值)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 错误消息 (仅错误时有值)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 追踪 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<String>,
}

impl<T> ApiResponse<T> {
    /// 创建成功响应
    pub fn success(data: T) -> Self {
        Self {
            code: 0,
            data: Some(data),
            reason: None,
            message: None,
            track: None,
        }
    }

    /// 创建错误响应
    pub fn error(code: i32, reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            data: None,
            reason: Some(reason.into()),
            message: Some(message.into()),
            track: None,
        }
    }

    /// 创建带追踪 ID 的错误响应
    pub fn error_with_track(
        code: i32,
        reason: impl Into<String>,
        message: impl Into<String>,
        track: impl Into<String>,
    ) -> Self {
        Self {
            code,
            data: None,
            reason: Some(reason.into()),
            message: Some(message.into()),
            track: Some(track.into()),
        }
    }

    /// 判断是否成功
    pub fn is_success(&self) -> bool {
        self.code == 0
    }

    /// 判断是否错误
    pub fn is_error(&self) -> bool {
        self.code > 0
    }

    /// 从 ErrorCode 创建错误响应
    pub fn from_code(code: flare_core_base::error::ErrorCode) -> Self {
        Self {
            code: code as i32,
            data: None,
            reason: Some(code.to_string()),
            message: Some(code.to_string()),
            track: None,
        }
    }
}

/// 从 FlareError 转换为 ApiResponse
///
/// 转换规则:
/// - 基础 HTTP 错误码 (10000-10999): 保持原样
/// - 其他错误码: 统一转换为 500 (HttpInternalServerError)
impl<T> From<FlareError> for ApiResponse<T> {
    fn from(err: FlareError) -> Self {
        let (original_code, reason, message) = match &err {
            FlareError::Localized {
                code,
                reason,
                details,
                ..
            } => (
                *code as i32,
                reason.clone(),
                details.clone().unwrap_or_default(),
            ),
            FlareError::System(msg) => (
                ErrorCode::InternalError as i32,
                "internal".to_string(),
                msg.clone(),
            ),
            FlareError::Io(msg) => (
                ErrorCode::NetworkError as i32,
                "network".to_string(),
                msg.clone(),
            ),
        };

        // 判断是否为 HTTP 基础错误码 (10000-10999)
        let final_code = if original_code >= 10000 && original_code <= 10999 {
            original_code
        } else {
            // 非 HTTP 基础错误码,统一转换为 500
            ErrorCode::HttpInternalServerError as i32
        };

        Self {
            code: final_code,
            data: None,
            reason: Some(reason),
            message: Some(message),
            track: None,
        }
    }
}

#[cfg(feature = "proto")]
impl<T> ApiResponse<T> {
    /// 从 ProtoErrorDetail 转换
    pub fn from_proto_error(detail: &ProtoErrorDetail) -> Self {
        // 判断是否为 HTTP 基础错误码
        let final_code = if detail.code >= 10000 && detail.code <= 10999 {
            detail.code
        } else {
            ErrorCode::HttpInternalServerError as i32
        };

        Self {
            code: final_code,
            data: None,
            reason: Some(detail.reason.clone()),
            message: Some(detail.message.clone()),
            track: Some(detail.track.clone()),
        }
    }

    /// 转换为 ProtoErrorDetail
    pub fn to_proto_error(&self) -> Option<ProtoErrorDetail> {
        if self.is_error() {
            Some(ProtoErrorDetail {
                code: self.code,
                reason: self.reason.clone().unwrap_or_default(),
                message: self.message.clone().unwrap_or_default(),
                track: self.track.clone().unwrap_or_default(),
            })
        } else {
            None
        }
    }
}

/// 快速转换宏: FlareError -> ApiResponse
#[macro_export]
macro_rules! flare_error_to_response {
    ($err:expr) => {
        $crate::http::response::ApiResponse::<()>::from($err)
    };
}

/// 快速转换宏(带类型)
#[macro_export]
macro_rules! flare_error_to_response_t {
    ($err:expr, $t:ty) => {
        $crate::http::response::ApiResponse::<$t>::from($err)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use flare_core_base::error::ErrorCode;

    #[test]
    fn test_success_response() {
        let response = ApiResponse::success("test data".to_string());
        assert!(response.is_success());
        assert!(!response.is_error());
        assert_eq!(response.code, 0);
        assert_eq!(response.data, Some("test data".to_string()));
        assert!(response.reason.is_none());
        assert!(response.message.is_none());
    }

    #[test]
    fn test_error_response() {
        let response: ApiResponse<String> = ApiResponse::error(
            ErrorCode::HttpBadRequest as i32,
            "BAD_REQUEST",
            "Invalid input",
        );
        assert!(!response.is_success());
        assert!(response.is_error());
        assert_eq!(response.code, ErrorCode::HttpBadRequest as i32);
        assert!(response.data.is_none());
        assert_eq!(response.reason, Some("BAD_REQUEST".to_string()));
        assert_eq!(response.message, Some("Invalid input".to_string()));
    }

    #[test]
    fn test_from_flare_error_with_http_code() {
        // HTTP 基础错误码应保持不变
        let err = FlareError::Localized {
            code: ErrorCode::HttpNotFound,
            reason: "not_found".to_string(),
            details: Some("Resource not found".to_string()),
            params: None,
            timestamp: chrono::Utc::now(),
        };

        let response: ApiResponse<String> = err.into();
        assert_eq!(response.code, ErrorCode::HttpNotFound as i32);
        assert_eq!(response.reason, Some("not_found".to_string()));
    }

    #[test]
    fn test_from_flare_error_with_non_http_code() {
        // 非 HTTP 错误码应转换为 500
        let err = FlareError::Localized {
            code: ErrorCode::InvalidArgument,
            reason: "validation".to_string(),
            details: Some("Field is required".to_string()),
            params: None,
            timestamp: chrono::Utc::now(),
        };

        let response: ApiResponse<String> = err.into();
        assert_eq!(response.code, ErrorCode::HttpInternalServerError as i32);
        assert_eq!(response.reason, Some("validation".to_string()));
    }

    #[test]
    fn test_from_flare_error_system() {
        let err = FlareError::System("Internal error".to_string());
        let response: ApiResponse<String> = err.into();
        assert_eq!(response.code, ErrorCode::HttpInternalServerError as i32);
    }
}
