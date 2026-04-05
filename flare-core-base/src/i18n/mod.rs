//! 国际化支持模块
//!
//! 提供错误信息的国际化功能，支持从文件加载翻译

use crate::error::LocalizedError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 国际化管理器
pub struct I18n {
    translations: Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
    default_locale: String,
}

impl I18n {
    /// 创建新的国际化管理器
    pub fn new(default_locale: impl Into<String>) -> Self {
        Self {
            translations: Arc::new(RwLock::new(HashMap::new())),
            default_locale: default_locale.into(),
        }
    }

    /// 从文件加载翻译
    pub async fn load_from_file(
        &self,
        locale: impl Into<String>,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let content = tokio::fs::read_to_string(file_path.as_ref()).await?;
        let translations: HashMap<String, String> = toml::from_str(&content)?;
        self.load_translations(locale, translations).await;
        Ok(())
    }

    /// 从 JSON 文件加载翻译
    pub async fn load_from_json(
        &self,
        locale: impl Into<String>,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let content = tokio::fs::read_to_string(file_path.as_ref()).await?;
        let translations: HashMap<String, String> = serde_json::from_str(&content)?;
        self.load_translations(locale, translations).await;
        Ok(())
    }

    /// 从目录加载所有翻译文件
    ///
    /// 支持的文件格式：
    /// - `{locale}.toml` (TOML 格式)
    /// - `{locale}.json` (JSON 格式)
    pub async fn load_from_dir(
        &self,
        dir_path: impl AsRef<std::path::Path>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut entries = tokio::fs::read_dir(dir_path.as_ref()).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                let file_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or("Invalid file name")?;

                let extension = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .ok_or("Invalid file extension")?;

                match extension {
                    "toml" => {
                        self.load_from_file(file_name, &path).await?;
                    }
                    "json" => {
                        self.load_from_json(file_name, &path).await?;
                    }
                    _ => {
                        tracing::warn!("Unsupported translation file format: {}", path.display());
                    }
                }
            }
        }

        Ok(())
    }

    /// 加载翻译
    pub async fn load_translations(
        &self,
        locale: impl Into<String>,
        translations: HashMap<String, String>,
    ) {
        let mut trans = self.translations.write().await;
        trans.insert(locale.into(), translations);
    }

    /// 获取翻译后的错误信息
    pub async fn translate_error(&self, error: &LocalizedError, locale: Option<&str>) -> String {
        let locale = locale.unwrap_or(&self.default_locale);
        let translations = self.translations.read().await;

        if let Some(locale_trans) = translations.get(locale) {
            // 尝试获取翻译
            let key = error.code_str();
            if let Some(translated) = locale_trans.get(key) {
                // 如果有参数，进行插值
                if let Some(ref params) = error.params {
                    return interpolate(translated, params);
                }
                return translated.clone();
            }
        }

        // 如果没有翻译，返回原始原因
        error.reason.clone()
    }

    /// 设置默认语言
    pub fn set_default_locale(&mut self, locale: impl Into<String>) {
        self.default_locale = locale.into();
    }

    /// 获取默认语言
    pub fn default_locale(&self) -> &str {
        &self.default_locale
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new("zh-CN")
    }
}

/// 插值函数：将参数替换到模板字符串中
fn interpolate(template: &str, params: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in params {
        let placeholder = format!("{{{}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

/// 预定义的错误信息翻译（中文）
pub fn default_zh_cn_translations() -> HashMap<String, String> {
    let mut translations = HashMap::new();

    // 连接相关
    translations.insert("CONNECTION_FAILED".to_string(), "连接失败".to_string());
    translations.insert("CONNECTION_TIMEOUT".to_string(), "连接超时".to_string());
    translations.insert("CONNECTION_CLOSED".to_string(), "连接已关闭".to_string());

    // 认证相关
    translations.insert("AUTHENTICATION_FAILED".to_string(), "认证失败".to_string());
    translations.insert(
        "AUTHENTICATION_EXPIRED".to_string(),
        "认证已过期".to_string(),
    );
    translations.insert("PERMISSION_DENIED".to_string(), "权限不足".to_string());

    // 用户相关
    translations.insert("USER_NOT_FOUND".to_string(), "用户不存在".to_string());
    translations.insert("USER_OFFLINE".to_string(), "用户离线".to_string());

    // 消息相关
    translations.insert(
        "MESSAGE_SEND_FAILED".to_string(),
        "消息发送失败".to_string(),
    );
    translations.insert("MESSAGE_NOT_FOUND".to_string(), "消息不存在".to_string());

    // 系统相关
    translations.insert("INTERNAL_ERROR".to_string(), "内部错误".to_string());
    translations.insert("SERVICE_UNAVAILABLE".to_string(), "服务不可用".to_string());

    translations
}

/// 预定义的错误信息翻译（英文）
pub fn default_en_us_translations() -> HashMap<String, String> {
    let mut translations = HashMap::new();

    // 连接相关
    translations.insert(
        "CONNECTION_FAILED".to_string(),
        "Connection failed".to_string(),
    );
    translations.insert(
        "CONNECTION_TIMEOUT".to_string(),
        "Connection timeout".to_string(),
    );
    translations.insert(
        "CONNECTION_CLOSED".to_string(),
        "Connection closed".to_string(),
    );

    // 认证相关
    translations.insert(
        "AUTHENTICATION_FAILED".to_string(),
        "Authentication failed".to_string(),
    );
    translations.insert(
        "AUTHENTICATION_EXPIRED".to_string(),
        "Authentication expired".to_string(),
    );
    translations.insert(
        "PERMISSION_DENIED".to_string(),
        "Permission denied".to_string(),
    );

    // 用户相关
    translations.insert("USER_NOT_FOUND".to_string(), "User not found".to_string());
    translations.insert("USER_OFFLINE".to_string(), "User offline".to_string());

    // 消息相关
    translations.insert(
        "MESSAGE_SEND_FAILED".to_string(),
        "Message send failed".to_string(),
    );
    translations.insert(
        "MESSAGE_NOT_FOUND".to_string(),
        "Message not found".to_string(),
    );

    // 系统相关
    translations.insert("INTERNAL_ERROR".to_string(), "Internal error".to_string());
    translations.insert(
        "SERVICE_UNAVAILABLE".to_string(),
        "Service unavailable".to_string(),
    );

    translations
}
