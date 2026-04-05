//! 错误构建器
//!
//! 提供链式 API 用于构建错误

use super::{ErrorCode, LocalizedError};

/// 错误构建器
pub struct ErrorBuilder {
    code: ErrorCode,
    reason: String,
    details: Option<String>,
    params: Option<std::collections::HashMap<String, String>>,
}

impl ErrorBuilder {
    /// 创建新的错误构建器
    pub fn new(code: ErrorCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
            details: None,
            params: None,
        }
    }

    /// 添加错误详情
    #[must_use]
    pub fn details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// 添加错误参数
    #[must_use]
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if self.params.is_none() {
            self.params = Some(std::collections::HashMap::new());
        }
        if let Some(ref mut params) = self.params {
            params.insert(key.into(), value.into());
        }
        self
    }

    /// 构建本地化错误
    pub fn build(self) -> LocalizedError {
        LocalizedError {
            code: self.code,
            reason: self.reason,
            details: self.details,
            params: self.params,
            timestamp: chrono::Utc::now(),
        }
    }

    /// 构建 FlareError
    pub fn build_error(self) -> super::FlareError {
        super::FlareError::Localized {
            code: self.code,
            reason: self.reason,
            details: self.details,
            params: self.params,
            timestamp: chrono::Utc::now(),
        }
    }
}
