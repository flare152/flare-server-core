//! MQ 生产者框架
//!
//! 提供统一的生产者接口，支持多种 MQ 实现。

use std::collections::HashMap;

use async_trait::async_trait;

/// 生产者 Trait
#[async_trait]
pub trait Producer: Send + Sync {
    /// 发送消息
    ///
    /// # 参数
    /// * `ctx` - 上下文对象（用于透传追踪信息）
    /// * `topic` - 目标主题
    /// * `key` - 消息键（可选，用于分区）
    /// * `payload` - 消息内容
    /// * `headers` - 消息头（可选）
    ///
    /// # 返回
    /// * `Result<(), ProducerError>` - 发送结果
    async fn send(
        &self,
        ctx: &flare_core_base::context::Ctx,
        topic: &str,
        key: Option<&str>,
        payload: Vec<u8>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), ProducerError>;

    /// 批量发送消息
    ///
    /// # 参数
    /// * `ctx` - 上下文对象（用于透传追踪信息）
    /// * `messages` - 消息列表
    ///
    /// # 返回
    /// * `Result<(), ProducerError>` - 发送结果
    async fn send_batch(
        &self,
        ctx: &flare_core_base::context::Ctx,
        messages: Vec<ProducerMessage>,
    ) -> Result<(), ProducerError>;

    /// 获取生产者名称
    fn name(&self) -> &str;
}

/// 生产者消息
#[derive(Debug, Clone)]
pub struct ProducerMessage {
    /// 目标主题
    pub topic: String,
    /// 消息键（可选，用于分区）
    pub key: Option<String>,
    /// 消息内容
    pub payload: Vec<u8>,
    /// 消息头
    pub headers: HashMap<String, String>,
}

impl ProducerMessage {
    /// 创建新的生产者消息
    pub fn new(topic: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            topic: topic.into(),
            key: None,
            payload,
            headers: HashMap::new(),
        }
    }

    /// 设置消息键
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// 添加消息头
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// 批量添加消息头
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = headers;
        self
    }
}

/// 生产者错误
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Send error: {0}")]
    Send(String),

    #[error("Batch error: {0}")]
    Batch(String),
}

/// 生产者配置
#[derive(Debug, Clone)]
pub struct ProducerConfig {
    /// 超时时间（毫秒）
    pub timeout_ms: u64,
    /// 是否启用幂等性
    pub enable_idempotence: bool,
    /// 压缩类型
    pub compression_type: String,
    /// 批处理大小（字节）
    pub batch_size: usize,
    /// 延迟时间（毫秒）
    pub linger_ms: u64,
    /// 重试次数
    pub retries: u32,
    /// 重试退避时间（毫秒）
    pub retry_backoff_ms: u64,
}

impl Default for ProducerConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            enable_idempotence: true,
            compression_type: "snappy".to_string(),
            batch_size: 64 * 1024,
            linger_ms: 10,
            retries: 3,
            retry_backoff_ms: 100,
        }
    }
}

impl ProducerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_idempotence(mut self, enable: bool) -> Self {
        self.enable_idempotence = enable;
        self
    }

    pub fn with_compression(mut self, compression: impl Into<String>) -> Self {
        self.compression_type = compression.into();
        self
    }

    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    pub fn with_linger_ms(mut self, linger_ms: u64) -> Self {
        self.linger_ms = linger_ms;
        self
    }

    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }
}
