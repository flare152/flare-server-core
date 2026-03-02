//! KV存储实现

use std::sync::Arc;

use crate::kv::{KvBackend, KvEntry, KvError};

/// KV存储实现
pub struct KvStore {
    backend: Arc<dyn KvBackend>,
}

impl KvStore {
    /// 创建新的KV存储
    pub fn new(backend: Arc<dyn KvBackend>) -> Self {
        Self { backend }
    }
    
    /// 获取键值
    pub async fn get(&self, key: &str) -> Result<Option<KvEntry>, KvError> {
        self.backend.get(key).await
    }
    
    /// 获取键值并转换为字符串
    pub async fn get_string(&self, key: &str) -> Result<Option<String>, KvError> {
        match self.backend.get(key).await? {
            Some(entry) => {
                String::from_utf8(entry.value)
                    .map(Some)
                    .map_err(|e| KvError::OperationFailed(format!("Failed to convert value to string: {}", e)))
            }
            None => Ok(None),
        }
    }
    
    /// 设置键值
    pub async fn put(&self, key: &str, value: &[u8]) -> Result<(), KvError> {
        self.backend.put(key, value).await
    }
    
    /// 设置键值（字符串形式）
    pub async fn put_string(&self, key: &str, value: &str) -> Result<(), KvError> {
        self.backend.put(key, value.as_bytes()).await
    }
    
    /// 删除键
    pub async fn delete(&self, key: &str) -> Result<bool, KvError> {
        self.backend.delete(key).await
    }
}