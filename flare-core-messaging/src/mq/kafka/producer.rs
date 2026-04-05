//! Kafka 生产者实现
//!
//! 实现基于 Kafka 的消息生产，支持 Context 透传。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::FutureProducer;
use rdkafka::producer::FutureRecord;

use super::config::KafkaProducerConfig;
use crate::mq::producer::{Producer, ProducerError, ProducerMessage};

/// Kafka 生产者
pub struct KafkaProducer {
    producer: Arc<FutureProducer>,
    config: crate::mq::producer::ProducerConfig,
}

impl KafkaProducer {
    /// 创建新的 Kafka 生产者
    pub fn new<C>(config: &C) -> Result<Self, ProducerError>
    where
        C: KafkaProducerConfig + Send + Sync,
    {
        let producer = super::build_kafka_producer(config)
            .map_err(|e| ProducerError::Configuration(e.to_string()))?;

        let producer_config = crate::mq::producer::ProducerConfig {
            timeout_ms: config.message_timeout_ms(),
            enable_idempotence: config.enable_idempotence(),
            compression_type: config.compression_type().to_string(),
            batch_size: config.batch_size(),
            linger_ms: config.linger_ms(),
            retries: config.retries(),
            retry_backoff_ms: config.retry_backoff_ms(),
        };

        Ok(Self {
            producer: Arc::new(producer),
            config: producer_config,
        })
    }

    /// 添加 Context 到消息头（实现 Context 透传）
    ///
    /// 使用统一的 `mq::context::merge_ctx_to_headers` 方法
    fn add_context_to_headers(
        &self,
        headers: &mut HashMap<String, String>,
        ctx: &flare_core_base::context::Ctx,
    ) {
        crate::mq::context::merge_ctx_to_headers(headers, ctx);
    }

    /// 生成消息 ID
    fn generate_message_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[async_trait::async_trait]
impl Producer for KafkaProducer {
    async fn send(
        &self,
        ctx: &flare_core_base::context::Ctx,
        topic: &str,
        key: Option<&str>,
        payload: Vec<u8>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), ProducerError> {
        let mut all_headers = headers.unwrap_or_default();
        self.add_context_to_headers(&mut all_headers, ctx);
        let message_id = self.generate_message_id();

        let mut owned = OwnedHeaders::new();
        owned = owned.insert(Header {
            key: "x-message-id",
            value: Some(message_id.as_str()),
        });
        for (hk, hv) in all_headers {
            owned = owned.insert(Header {
                key: hk.as_str(),
                value: Some(hv.as_str()),
            });
        }

        let mut record = FutureRecord::to(topic).payload(&payload).headers(owned);

        if let Some(k) = key {
            record = record.key(k);
        }

        self.producer
            .send(record, Duration::from_millis(self.config.timeout_ms))
            .await
            .map_err(|(e, _)| ProducerError::Send(e.to_string()))?;

        tracing::debug!(
            topic = %topic,
            message_id = %message_id,
            trace_id = %ctx.trace_id(),
            "Message sent successfully"
        );

        Ok(())
    }

    async fn send_batch(
        &self,
        ctx: &flare_core_base::context::Ctx,
        messages: Vec<ProducerMessage>,
    ) -> Result<(), ProducerError> {
        for msg in messages {
            self.send(
                ctx,
                &msg.topic,
                msg.key.as_deref(),
                msg.payload,
                Some(msg.headers),
            )
            .await?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "kafka_producer"
    }
}

/// Kafka 生产者构建器
pub struct KafkaProducerBuilder {
    config: crate::mq::producer::ProducerConfig,
}

impl KafkaProducerBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            config: crate::mq::producer::ProducerConfig::default(),
        }
    }

    /// 设置配置
    pub fn with_config(mut self, config: crate::mq::producer::ProducerConfig) -> Self {
        self.config = config;
        self
    }

    /// 构建 Kafka 生产者
    pub fn build<C>(self, kafka_config: &C) -> Result<KafkaProducer, ProducerError>
    where
        C: KafkaProducerConfig + Send + Sync,
    {
        KafkaProducer::new(kafka_config)
    }
}

impl Default for KafkaProducerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
