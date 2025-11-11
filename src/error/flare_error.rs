//! Flare IM 统一错误类型
//!
//! 与 flare-core 中的 FlareError 定义完全一致

use super::code::ErrorCode;
use super::localized::LocalizedError;
use std::collections::HashMap;
use thiserror::Error;

/// Flare IM 统一错误类型
#[derive(Error, Debug, Clone)]
pub enum FlareError {
    /// 本地化错误（用于暴露给用户）
    #[error("错误 [{code}] {reason}", code = .code.as_str())]
    Localized {
        code: ErrorCode,
        reason: String,
        details: Option<String>,
        params: Option<HashMap<String, String>>,
        timestamp: chrono::DateTime<chrono::Utc>,
    },

    /// 系统错误（用于内部错误，不暴露给用户）
    #[error("系统错误: {0}")]
    System(String),

    /// IO 错误
    #[error("IO 错误: {0}")]
    Io(String),
}

impl FlareError {
    /// 创建本地化错误
    pub fn localized(code: ErrorCode, reason: impl Into<String>) -> Self {
        FlareError::Localized {
            code,
            reason: reason.into(),
            details: None,
            params: None,
            timestamp: chrono::Utc::now(),
        }
    }

    /// 创建系统错误
    pub fn system(msg: impl Into<String>) -> Self {
        FlareError::System(msg.into())
    }

    /// 创建 IO 错误
    pub fn io(msg: impl Into<String>) -> Self {
        FlareError::Io(msg.into())
    }

    // ============================================================
    // 便捷方法：连接相关错误
    // ============================================================

    /// 创建连接失败错误
    pub fn connection_failed(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::ConnectionFailed, reason)
    }

    /// 创建连接超时错误
    pub fn connection_timeout(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::ConnectionTimeout, reason)
    }

    /// 创建连接已关闭错误
    pub fn connection_closed(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::ConnectionClosed, reason)
    }

    // ============================================================
    // 便捷方法：认证相关错误
    // ============================================================

    /// 创建认证失败错误
    pub fn authentication_failed(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::AuthenticationFailed, reason)
    }

    /// 创建认证过期错误
    pub fn authentication_expired(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::AuthenticationExpired, reason)
    }

    // ============================================================
    // 便捷方法：协议相关错误
    // ============================================================

    /// 创建协议错误
    pub fn protocol_error(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::ProtocolError, reason)
    }

    /// 创建消息格式错误
    pub fn message_format_error(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::MessageFormatError, reason)
    }

    // ============================================================
    // 便捷方法：消息相关错误
    // ============================================================

    /// 创建消息发送失败错误
    pub fn message_send_failed(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::MessageSendFailed, reason)
    }

    // ============================================================
    // 便捷方法：用户相关错误
    // ============================================================

    /// 创建用户不存在错误
    pub fn user_not_found(user_id: impl Into<String>) -> Self {
        let mut params = HashMap::new();
        params.insert("user_id".to_string(), user_id.into());
        Self::Localized {
            code: ErrorCode::UserNotFound,
            reason: "用户不存在".to_string(),
            details: None,
            params: Some(params),
            timestamp: chrono::Utc::now(),
        }
    }

    /// 创建用户离线错误
    pub fn user_offline(user_id: impl Into<String>) -> Self {
        let mut params = HashMap::new();
        params.insert("user_id".to_string(), user_id.into());
        Self::Localized {
            code: ErrorCode::UserOffline,
            reason: "用户离线".to_string(),
            details: None,
            params: Some(params),
            timestamp: chrono::Utc::now(),
        }
    }

    // ============================================================
    // 便捷方法：序列化相关错误
    // ============================================================

    /// 创建序列化错误
    pub fn serialization_error(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::SerializationError, reason)
    }

    /// 创建反序列化错误
    pub fn deserialization_error(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::DeserializationError, reason)
    }

    /// 创建编码错误
    pub fn encoding_error(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::EncodingError, reason)
    }

    // ============================================================
    // 便捷方法：通用错误
    // ============================================================

    /// 创建通用错误
    pub fn general_error(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::GeneralError, reason)
    }

    /// 创建操作超时错误
    pub fn timeout(reason: impl Into<String>) -> Self {
        Self::localized(ErrorCode::OperationTimeout, reason)
    }

    // ============================================================
    // 信息获取方法
    // ============================================================

    /// 获取本地化错误信息（如果有）
    pub fn as_localized(&self) -> Option<LocalizedError> {
        match self {
            FlareError::Localized {
                code,
                reason,
                details,
                params,
                timestamp,
            } => Some(LocalizedError {
                code: *code,
                reason: reason.clone(),
                details: details.clone(),
                params: params.clone(),
                timestamp: *timestamp,
            }),
            _ => None,
        }
    }

    /// 获取错误代码
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            FlareError::Localized { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// 获取错误原因
    pub fn reason(&self) -> &str {
        match self {
            FlareError::Localized { reason, .. } => reason,
            FlareError::System(msg) => msg,
            FlareError::Io(msg) => msg,
        }
    }

    /// 转换为本地化错误
    pub fn to_localized(self) -> LocalizedError {
        match self {
            FlareError::Localized {
                code,
                reason,
                details,
                params,
                timestamp,
            } => LocalizedError {
                code,
                reason,
                details,
                params,
                timestamp,
            },
            FlareError::System(msg) => LocalizedError::new(ErrorCode::InternalError, msg),
            FlareError::Io(msg) => LocalizedError::new(ErrorCode::NetworkError, msg),
        }
    }

    /// 判断是否为可重试的错误
    pub fn is_retryable(&self) -> bool {
        self.code().map(|code| code.is_retryable()).unwrap_or(false)
    }
}

/// 服务端错误类型别名
pub type ServerError = FlareError;

/// 结果类型别名
pub type Result<T> = std::result::Result<T, FlareError>;
