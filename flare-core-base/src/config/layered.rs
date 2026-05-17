use std::path::Path;

use serde::de::DeserializeOwned;
use toml::Value as TomlValue;

/// 分层配置读取器：环境变量优先，TOML 兜底。
///
/// 目标是抽离“读取机制”，不绑定具体业务结构体。
#[derive(Debug, Clone)]
pub struct LayeredConfig {
    doc: TomlValue,
}

impl Default for LayeredConfig {
    fn default() -> Self {
        Self {
            doc: TomlValue::Table(toml::map::Map::new()),
        }
    }
}

impl LayeredConfig {
    /// 从可选 TOML 文件构建读取器；读取失败时降级为空表并记录日志。
    pub fn from_optional_toml(path: Option<&Path>) -> Self {
        let mut doc = TomlValue::Table(toml::map::Map::new());
        if let Some(path) = path {
            match std::fs::read_to_string(path) {
                Ok(raw) => match toml::from_str::<TomlValue>(&raw) {
                    Ok(v) => doc = v,
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "failed to parse toml config, fallback to env/default"
                    ),
                },
                Err(e) => tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "toml config file not readable, fallback to env/default"
                ),
            }
        }
        Self { doc }
    }

    pub fn resolve_usize(&self, env_key: &str, toml_path: &str) -> Option<usize> {
        std::env::var(env_key)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .or_else(|| {
                self.toml_integer(toml_path)
                    .and_then(|n| usize::try_from(n).ok())
            })
    }

    pub fn resolve_u64(&self, env_key: &str, toml_path: &str) -> Option<u64> {
        std::env::var(env_key)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| {
                self.toml_integer(toml_path)
                    .and_then(|n| u64::try_from(n).ok())
            })
    }

    pub fn resolve_u32(&self, env_key: &str, toml_path: &str) -> Option<u32> {
        std::env::var(env_key)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .or_else(|| {
                self.toml_integer(toml_path)
                    .and_then(|n| u32::try_from(n).ok())
            })
    }

    pub fn resolve_bool(&self, env_key: &str, toml_path: &str) -> Option<bool> {
        std::env::var(env_key)
            .ok()
            .and_then(|raw| parse_bool(raw.as_str()))
            .or_else(|| self.value_at(toml_path).and_then(|v| v.as_bool()))
    }

    pub fn resolve_nonempty_string(&self, env_key: &str, toml_path: &str) -> Option<String> {
        std::env::var(env_key)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.value_at(toml_path)
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
    }

    pub fn resolve_json_vec_or_toml_vec<T>(
        &self,
        env_json_key: &str,
        toml_path: &str,
    ) -> Option<Vec<T>>
    where
        T: DeserializeOwned,
    {
        if let Some(raw) = std::env::var(env_json_key).ok() {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Some(vec![]);
            }
            return serde_json::from_str::<Vec<T>>(trimmed)
                .map_err(|e| {
                    tracing::error!(
                        env = %env_json_key,
                        error = %e,
                        "invalid json env value, ignored"
                    );
                    e
                })
                .ok();
        }

        self.deserialize_toml_path::<Vec<T>>(toml_path)
    }

    pub fn deserialize_toml_path<T>(&self, toml_path: &str) -> Option<T>
    where
        T: DeserializeOwned,
    {
        let value = self.value_at(toml_path)?.clone();
        value.try_into().ok()
    }

    fn toml_integer(&self, toml_path: &str) -> Option<i64> {
        self.value_at(toml_path).and_then(|v| v.as_integer())
    }

    fn value_at(&self, toml_path: &str) -> Option<&TomlValue> {
        let mut current = &self.doc;
        for seg in toml_path.split('.') {
            current = current.get(seg)?;
        }
        Some(current)
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    let v = raw.trim();
    if matches!(v, "1" | "true" | "yes" | "on") || v.eq_ignore_ascii_case("true") {
        return Some(true);
    }
    if matches!(v, "0" | "false" | "no" | "off") || v.eq_ignore_ascii_case("false") {
        return Some(false);
    }
    None
}
