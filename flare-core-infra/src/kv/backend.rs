//! KV存储后端抽象

use async_trait::async_trait;

/// KV存储条目
#[derive(Debug, Clone)]
pub struct KvEntry {
    /// 键
    pub key: String,
    /// 值
    pub value: Vec<u8>,
    /// 版本
    pub version: u64,
    /// 创建修订版本
    pub create_revision: u64,
    /// 修改修订版本
    pub mod_revision: u64,
    /// 租约ID
    pub lease: i64,
}

/// KV存储错误
#[derive(Debug, thiserror::Error)]
pub enum KvError {
    #[error("KV operation failed: {0}")]
    OperationFailed(String),
    #[error("Key not found: {0}")]
    KeyNotFound(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}

/// KV存储后端 trait
#[async_trait]
pub trait KvBackend: Send + Sync {
    /// 获取键值
    async fn get(&self, key: &str) -> Result<Option<KvEntry>, KvError>;

    /// 获取多个键值
    async fn get_range(&self, key: &str, range_end: &str) -> Result<Vec<KvEntry>, KvError>;

    /// 设置键值
    async fn put(&self, key: &str, value: &[u8]) -> Result<(), KvError>;

    /// 删除键
    async fn delete(&self, key: &str) -> Result<bool, KvError>;

    /// 删除范围内的键
    async fn delete_range(&self, key: &str, range_end: &str) -> Result<u64, KvError>;

    /// 获取键的前缀列表
    async fn prefix_keys(&self, prefix: &str) -> Result<Vec<String>, KvError>;
}
