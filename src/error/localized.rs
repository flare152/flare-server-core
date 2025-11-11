//! 国际化错误信息结构
//!
//! 与 flare-core 中的 LocalizedError 定义完全一致

use super::code::{ErrorCategory, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// 国际化错误信息结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalizedError {
    /// 错误代码
    pub code: ErrorCode,
    /// 错误原因（用于国际化）
    pub reason: String,
    /// 错误详情（可选，用于调试）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// 错误参数（用于国际化插值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, String>>,
    /// 错误时间戳
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl LocalizedError {
    /// 创建新的本地化错误
    pub fn new(code: ErrorCode, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
            details: None,
            params: None,
            timestamp: chrono::Utc::now(),
        }
    }

    /// 添加错误详情
    #[must_use]
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// 添加错误参数
    #[must_use]
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self {
        self.params = Some(params);
        self
    }

    /// 添加单个参数
    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        if self.params.is_none() {
            self.params = Some(HashMap::new());
        }
        if let Some(ref mut params) = self.params {
            params.insert(key.into(), value.into());
        }
        self
    }

    /// 获取错误代码的数字值
    #[inline]
    pub fn code_value(&self) -> u32 {
        self.code.as_u32()
    }

    /// 获取错误代码的字符串标识符
    #[inline]
    pub fn code_str(&self) -> &'static str {
        self.code.as_str()
    }

    /// 获取错误类别
    #[inline]
    pub fn category(&self) -> ErrorCategory {
        self.code.category()
    }

    /// 判断是否为可重试的错误
    #[inline]
    pub fn is_retryable(&self) -> bool {
        self.code.is_retryable()
    }
}

impl fmt::Display for LocalizedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.reason)
    }
}
