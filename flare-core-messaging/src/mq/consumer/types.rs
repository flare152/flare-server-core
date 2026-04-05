//! MQ 消费者核心类型
//!
//! 定义通用的消息类型、错误类型等，不依赖具体的 MQ 实现。

use std::collections::HashMap;
use std::time::Instant;

use flare_core_base::context::Ctx;

/// 消息上下文
///
/// 包含消息的元数据信息和上下文信息
#[derive(Debug, Clone)]
pub struct MessageContext {
    /// 上下文对象（包含 trace_id、user_id 等）
    pub ctx: Ctx,
    /// 消息 ID
    pub message_id: String,
    /// 主题
    pub topic: String,
    /// 分区（可选，某些 MQ 支持）
    pub partition: i32,
    /// 偏移量（可选，某些 MQ 支持）
    pub offset: i64,
    /// 消息键（可选，用于分区）
    pub key: Option<String>,
    /// 消息头
    pub headers: HashMap<String, String>,
    /// 开始处理时间
    pub started_at: Instant,
    /// 重试次数
    pub retry_count: u32,
    /// 元数据
    pub metadata: HashMap<String, String>,
}

impl MessageContext {
    /// 创建新的消息上下文
    pub fn new(ctx: Ctx, topic: String) -> Self {
        Self {
            ctx,
            message_id: uuid::Uuid::new_v4().to_string(),
            topic,
            partition: 0,
            offset: 0,
            key: None,
            headers: HashMap::new(),
            started_at: Instant::now(),
            retry_count: 0,
            metadata: HashMap::new(),
        }
    }

    /// 获取处理耗时（毫秒）
    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// 设置消息键
    pub fn with_key(mut self, key: String) -> Self {
        self.key = Some(key);
        self
    }

    /// 添加消息头
    pub fn with_header(mut self, key: String, value: String) -> Self {
        self.headers.insert(key, value);
        self
    }

    /// 设置元数据
    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// 增加重试次数
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }
}

/// 消息处理结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageResult {
    /// 处理成功，确认消息
    Ack,
    /// 处理失败，拒绝消息（将返回队列）
    Nack,
    /// 处理失败，发送到 DLQ
    DeadLetter,
}

impl MessageResult {
    /// 是否需要确认消息
    pub fn should_ack(&self) -> bool {
        matches!(self, MessageResult::Ack)
    }
}

/// 消息处理错误
#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error("Handler error: {0}")]
    Handler(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Timeout error: {0}")]
    Timeout(String),

    #[error("DLQ error: {0}")]
    DeadLetter(String),

    #[error("Shutdown requested")]
    Shutdown,

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("No handler found for topic: {0}")]
    NoHandler(String),
}

impl ConsumerError {
    /// 是否可重试
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ConsumerError::Handler(_) | ConsumerError::Connection(_) | ConsumerError::Timeout(_)
        )
    }

    /// 是否应该发送到 DLQ
    pub fn should_dead_letter(&self) -> bool {
        !self.is_retryable() && !matches!(self, ConsumerError::Shutdown)
    }
}

/// 内容类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentType {
    /// JSON
    Json,
    /// Protobuf
    Protobuf,
    /// Avro
    Avro,
    /// 原始字节
    Raw,
}

impl ContentType {
    /// 从字符串解析内容类型
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "application/json" | "json" => Some(ContentType::Json),
            "application/protobuf" | "protobuf" => Some(ContentType::Protobuf),
            "application/avro" | "avro" => Some(ContentType::Avro),
            "application/octet-stream" | "raw" => Some(ContentType::Raw),
            _ => None,
        }
    }

    /// 转换为 MIME 类型字符串
    pub fn to_mime(&self) -> &'static str {
        match self {
            ContentType::Json => "application/json",
            ContentType::Protobuf => "application/protobuf",
            ContentType::Avro => "application/avro",
            ContentType::Raw => "application/octet-stream",
        }
    }
}

/// 通用的 MQ 消息类型
///
/// 支持多种序列化格式（JSON、Protobuf、Avro 等）
#[derive(Debug, Clone)]
pub struct Message {
    /// 原始消息内容
    pub payload: Vec<u8>,
    /// 序列化格式
    pub content_type: ContentType,
    /// 消息上下文
    pub context: MessageContext,
}

impl Message {
    /// 创建新的消息
    pub fn new(payload: Vec<u8>, content_type: ContentType, context: MessageContext) -> Self {
        Self {
            payload,
            content_type,
            context,
        }
    }

    /// 反序列化为 JSON（带格式校验）
    ///
    /// 会校验 content-type 是否为 JSON，用于防御性编程
    pub fn to_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConsumerError> {
        if self.content_type != ContentType::Json {
            return Err(ConsumerError::Deserialization(format!(
                "Expected JSON, got {:?}",
                self.content_type
            )));
        }

        serde_json::from_slice(&self.payload).map_err(|e| {
            ConsumerError::Deserialization(format!("JSON deserialization failed: {}", e))
        })
    }

    /// 直接反序列化为 JSON（不校验格式）
    ///
    /// 推荐使用此方法，因为：
    /// 1. payload 已经是序列化好的数据，直接解码即可
    /// 2. 避免不必要的 content-type 校验开销
    /// 3. 如果解码失败，serde_json 本身会返回错误
    ///
    /// # 示例
    /// ```rust
    /// let data = message.decode_json::<MyData>()?;
    /// ```
    pub fn decode_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConsumerError> {
        serde_json::from_slice(&self.payload).map_err(|e| {
            ConsumerError::Deserialization(format!("JSON deserialization failed: {}", e))
        })
    }

    /// 反序列化为 Protobuf（带格式校验）
    ///
    /// 会校验 content-type 是否为 Protobuf，用于防御性编程
    pub fn to_protobuf<T: prost::Message + Default>(&self) -> Result<T, ConsumerError> {
        if self.content_type != ContentType::Protobuf {
            return Err(ConsumerError::Deserialization(format!(
                "Expected Protobuf, got {:?}",
                self.content_type
            )));
        }

        T::decode(&*self.payload).map_err(|e| {
            ConsumerError::Deserialization(format!("Protobuf deserialization failed: {}", e))
        })
    }

    /// 直接反序列化为 Protobuf（不校验格式）
    ///
    /// 推荐使用此方法，因为：
    /// 1. payload 已经是序列化好的二进制，直接解码即可
    /// 2. 避免不必要的 content-type 校验开销
    /// 3. 如果解码失败，Protobuf 本身会返回错误
    ///
    /// # 示例
    /// ```rust
    /// let envelope = message.decode_protobuf::<MqEnvelope>()?;
    /// ```
    pub fn decode_protobuf<T: prost::Message + Default>(&self) -> Result<T, ConsumerError> {
        T::decode(&*self.payload).map_err(|e| {
            ConsumerError::Deserialization(format!("Protobuf deserialization failed: {}", e))
        })
    }

    /// 获取原始字符串
    pub fn to_string(&self) -> Result<String, ConsumerError> {
        String::from_utf8(self.payload.clone()).map_err(|e| {
            ConsumerError::Deserialization(format!("Failed to convert to string: {}", e))
        })
    }
}
