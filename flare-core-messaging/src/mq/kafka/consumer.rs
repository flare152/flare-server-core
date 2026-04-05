//! Kafka 消费者实现
//!
//! 实现基于 Kafka 的消息消费，支持并发消费、顺序消费、Context 透传等特性。

use std::collections::HashMap;
use std::sync::Arc;

use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{BorrowedMessage, Headers, Message as KafkaMessage};

use super::config::KafkaConsumerConfig;
use crate::mq::consumer::dispatcher::Dispatcher;
use crate::mq::consumer::{
    ConsumerConfig, ConsumerError, ContentType, Message, MessageContext, MessageFetcher,
};

/// Kafka 消息获取器
pub struct KafkaMessageFetcher {
    consumer: StreamConsumer,
    topics: Vec<String>,
    auto_commit: bool,
}

impl KafkaMessageFetcher {
    /// 创建新的 Kafka 消息获取器（`group.id` 来自 `config.consumer_group()`）
    pub fn new<C: KafkaConsumerConfig>(
        config: &C,
        topics: Vec<String>,
    ) -> Result<Self, ConsumerError> {
        Self::new_with_consumer_group(config, topics, None)
    }

    /// 创建 Kafka 消息获取器，并可选用 `consumer_group` 覆盖配置中的 `group.id`
    pub fn new_with_consumer_group<C: KafkaConsumerConfig>(
        config: &C,
        topics: Vec<String>,
        consumer_group: Option<&str>,
    ) -> Result<Self, ConsumerError> {
        let consumer = match consumer_group {
            Some(group_id) => {
                let cfg = super::config::KafkaConsumerGroupOverride {
                    inner: config,
                    group_id,
                };
                super::build_kafka_consumer(&cfg)
            }
            None => super::build_kafka_consumer(config),
        }
        .map_err(|e| ConsumerError::Connection(e.to_string()))?;

        let auto_commit = config.enable_auto_commit();
        let fetcher = Self {
            consumer,
            topics,
            auto_commit,
        };

        fetcher.subscribe()?;

        Ok(fetcher)
    }

    /// 订阅主题
    fn subscribe(&self) -> Result<(), ConsumerError> {
        let topics: Vec<&str> = self.topics.iter().map(|s| s.as_str()).collect();
        self.consumer
            .subscribe(&topics)
            .map_err(|e| ConsumerError::Connection(e.to_string()))?;

        tracing::info!(topics = ?topics, "Subscribed to Kafka topics");
        Ok(())
    }

    fn decode_message(&self, msg: &BorrowedMessage<'_>) -> Result<Message, ConsumerError> {
        message_from_kafka_borrowed(msg)
    }
}

#[async_trait::async_trait]
impl MessageFetcher for KafkaMessageFetcher {
    async fn fetch(&mut self) -> Result<Option<Message>, ConsumerError> {
        match self.consumer.recv().await {
            Ok(borrowed_msg) => {
                let message = self.decode_message(&borrowed_msg)?;

                // 手动提交 offset
                if !self.auto_commit {
                    if let Err(e) = self
                        .consumer
                        .commit_message(&borrowed_msg, CommitMode::Async)
                    {
                        tracing::warn!(error = %e, "Failed to commit offset");
                    }
                }

                Ok(Some(message))
            }
            Err(e) => {
                if e.to_string().contains("Shutdown") {
                    Ok(None)
                } else {
                    Err(ConsumerError::Connection(e.to_string()))
                }
            }
        }
    }
}

/// 从 Kafka 字符串消息头恢复 [flare_core_base::context::Ctx]（与生产者 `KafkaProducer::add_context_to_headers` 对称）
///
/// 使用统一的 `mq::context::mq_headers_to_ctx` 方法
pub fn context_from_kafka_headers(
    headers: &HashMap<String, String>,
) -> flare_core_base::context::Ctx {
    crate::mq::context::mq_headers_to_ctx(headers)
}

/// 将单条 `BorrowedMessage` 解码为统一 [Message]（含 payload、content-type 与透传的 Context）
pub fn message_from_kafka_borrowed(msg: &BorrowedMessage<'_>) -> Result<Message, ConsumerError> {
    let payload = msg.payload().unwrap_or(&[]);
    let topic = msg.topic();
    let partition = msg.partition();
    let offset = msg.offset();

    let mut headers = HashMap::new();
    if let Some(kafka_headers) = msg.headers() {
        for h in kafka_headers.iter() {
            let value_str = h
                .value
                .map(|v| std::str::from_utf8(v).unwrap_or("").to_string())
                .unwrap_or_default();
            headers.insert(h.key.to_string(), value_str);
        }
    }

    let key = msg
        .key()
        .map(|k| std::str::from_utf8(k).unwrap_or("").to_string());

    let message_id = headers
        .get("x-message-id")
        .cloned()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let ctx = context_from_kafka_headers(&headers);

    let content_type = headers
        .get("content-type")
        .and_then(|v| ContentType::from_str(v))
        .unwrap_or(ContentType::Json);

    let message_context = MessageContext {
        ctx,
        message_id,
        topic: topic.to_string(),
        partition,
        offset,
        key,
        headers,
        started_at: std::time::Instant::now(),
        retry_count: 0,
        metadata: HashMap::new(),
    };

    Ok(Message::new(
        payload.to_vec(),
        content_type,
        message_context,
    ))
}

/// Kafka 消费者运行时
///
/// 封装了 Kafka 特定的消费者逻辑
pub struct KafkaConsumerRuntime {
    config: ConsumerConfig,
    dispatcher: Arc<dyn crate::mq::consumer::Dispatcher>,
}

impl KafkaConsumerRuntime {
    /// 创建新的 Kafka 消费者运行时
    pub fn new(
        config: ConsumerConfig,
        dispatcher: Arc<dyn crate::mq::consumer::Dispatcher>,
    ) -> Self {
        Self { config, dispatcher }
    }

    /// 启动 Kafka 消费者
    pub async fn start<C>(&self, kafka_config: C) -> Result<(), ConsumerError>
    where
        C: KafkaConsumerConfig + Send + Sync + 'static,
    {
        let topics = self.dispatcher.topics();
        if topics.is_empty() {
            return Err(ConsumerError::Configuration(
                "No topics registered in dispatcher".to_string(),
            ));
        }

        tracing::info!(topics = ?topics, "Starting Kafka consumer");

        let fetcher = KafkaMessageFetcher::new_with_consumer_group(
            &kafka_config,
            topics,
            self.config.kafka_consumer_group_override.as_deref(),
        )?;
        let runtime =
            crate::mq::consumer::ConsumerRuntime::new(self.config.clone(), self.dispatcher.clone());

        let mut fetcher = fetcher;
        runtime.run(&mut fetcher).await
    }
}

/// Kafka 消费者构建器
///
/// 提供便捷的构建方式来创建 Kafka 消费者
pub struct KafkaConsumerBuilder {
    config: ConsumerConfig,
    dispatcher: crate::mq::consumer::TopicDispatcher,
}

impl KafkaConsumerBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            config: ConsumerConfig::default(),
            dispatcher: crate::mq::consumer::TopicDispatcher::new(),
        }
    }

    /// 设置配置
    pub fn with_config(mut self, config: ConsumerConfig) -> Self {
        self.config = config;
        self
    }

    /// 注册 Handler
    pub fn register_handler<H>(
        mut self,
        handler: H,
        topics: Vec<String>,
    ) -> Result<Self, ConsumerError>
    where
        H: crate::mq::consumer::MessageHandler + 'static,
    {
        let handler_arc: Arc<dyn crate::mq::consumer::MessageHandler> = Arc::new(handler);

        for topic in topics {
            Dispatcher::register(&mut self.dispatcher, topic, handler_arc.clone())?;
        }

        Ok(self)
    }

    /// 构建运行时
    pub fn build(self) -> KafkaConsumerRuntime {
        KafkaConsumerRuntime::new(self.config, Arc::new(self.dispatcher))
    }
}

impl Default for KafkaConsumerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
