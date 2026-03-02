//! 事件总线 Kafka 后端：按 partition_key 分区、consumer group、at-least-once
//!
//! 复用 `event_bus::kafka` 的生产者构建与配置约定，支持与通用 Kafka 一致的丰富配置。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::Stream;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::BorrowedMessage;
use rdkafka::producer::FutureRecord;
use rdkafka::Message;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, warn};

use crate::event_bus::envelope::EventEnvelope;
use crate::event_bus::kafka::{build_kafka_producer, KafkaProducerConfig};
use crate::event_bus::topic_bus::{TopicEventBus, TopicEventBusError};

/// 事件总线 Kafka 后端配置（支持与通用 Kafka 一致的丰富选项）
#[derive(Clone, Debug)]
pub struct KafkaEventBusConfig {
    pub bootstrap_servers: String,
    pub topic: String,
    pub consumer_group: String,
    pub timeout_ms: u64,
    pub enable_auto_commit: bool,
    // 生产者（与 KafkaProducerConfig 对齐）
    pub compression_type: String,
    pub batch_size: usize,
    pub linger_ms: u64,
    pub retries: u32,
    pub retry_backoff_ms: u64,
    pub metadata_max_age_ms: u64,
    // 消费者
    pub session_timeout_ms: u64,
    pub auto_offset_reset: String,
    pub fetch_min_bytes: usize,
    pub fetch_max_wait_ms: u64,
    pub fetch_message_max_bytes: usize,
    pub max_partition_fetch_bytes: usize,
}

impl Default for KafkaEventBusConfig {
    fn default() -> Self {
        Self {
            bootstrap_servers: "127.0.0.1:29092".to_string(),
            topic: "flare.im.message.events".to_string(),
            consumer_group: "default-group".to_string(),
            timeout_ms: 5000,
            enable_auto_commit: false,
            compression_type: "snappy".to_string(),
            batch_size: 64 * 1024,
            linger_ms: 10,
            retries: 3,
            retry_backoff_ms: 100,
            metadata_max_age_ms: 300_000,
            session_timeout_ms: 30_000,
            auto_offset_reset: "earliest".to_string(),
            fetch_min_bytes: 1,
            fetch_max_wait_ms: 500,
            fetch_message_max_bytes: 10 * 1024 * 1024,
            max_partition_fetch_bytes: 10 * 1024 * 1024,
        }
    }
}

impl KafkaEventBusConfig {
    /// 从必填项构建，其余使用默认值；可按需修改字段
    pub fn new(
        bootstrap_servers: impl Into<String>,
        topic: impl Into<String>,
        consumer_group: impl Into<String>,
    ) -> Self {
        Self {
            bootstrap_servers: bootstrap_servers.into(),
            topic: topic.into(),
            consumer_group: consumer_group.into(),
            ..Default::default()
        }
    }
}

impl KafkaProducerConfig for KafkaEventBusConfig {
    fn kafka_bootstrap(&self) -> &str {
        &self.bootstrap_servers
    }
    fn message_timeout_ms(&self) -> u64 {
        self.timeout_ms
    }
    fn enable_idempotence(&self) -> bool {
        true
    }
    fn compression_type(&self) -> &str {
        &self.compression_type
    }
    fn batch_size(&self) -> usize {
        self.batch_size
    }
    fn linger_ms(&self) -> u64 {
        self.linger_ms
    }
    fn retries(&self) -> u32 {
        self.retries
    }
    fn retry_backoff_ms(&self) -> u64 {
        self.retry_backoff_ms
    }
    fn metadata_max_age_ms(&self) -> u64 {
        self.metadata_max_age_ms
    }
}

/// Kafka 实现的 TopicEventBus
pub struct KafkaTopicEventBus {
    producer: Arc<rdkafka::producer::FutureProducer>,
    config: KafkaEventBusConfig,
}

impl KafkaTopicEventBus {
    pub fn new(config: KafkaEventBusConfig) -> Result<Arc<Self>, TopicEventBusError> {
        let producer = build_kafka_producer(&config)
            .map_err(|e| TopicEventBusError::Publish(e.to_string()))?;
        Ok(Arc::new(Self {
            producer: Arc::new(producer),
            config,
        }))
    }

    fn create_consumer(
        config: &KafkaEventBusConfig,
        consumer_group: &str,
    ) -> Result<StreamConsumer, TopicEventBusError> {
        use rdkafka::config::ClientConfig;
        ClientConfig::new()
            .set("bootstrap.servers", &config.bootstrap_servers)
            .set("group.id", consumer_group)
            .set("enable.auto.commit", if config.enable_auto_commit { "true" } else { "false" })
            .set("auto.offset.reset", &config.auto_offset_reset)
            .set("session.timeout.ms", &config.session_timeout_ms.to_string())
            .set("fetch.min.bytes", &config.fetch_min_bytes.to_string())
            .set("fetch.wait.max.ms", &config.fetch_max_wait_ms.to_string())
            .set("fetch.message.max.bytes", &config.fetch_message_max_bytes.to_string())
            .set("max.partition.fetch.bytes", &config.max_partition_fetch_bytes.to_string())
            .set("metadata.max.age.ms", &config.metadata_max_age_ms.to_string())
            .create()
            .map_err(|e| TopicEventBusError::Subscribe(e.to_string()))
    }
}

#[async_trait]
impl TopicEventBus for KafkaTopicEventBus {
    async fn publish(&self, topic: &str, envelope: &EventEnvelope) -> Result<(), TopicEventBusError> {
        let payload =
            serde_json::to_vec(envelope).map_err(|e| TopicEventBusError::Serialization(e.to_string()))?;
        let key = envelope.partition_key();
        let record = FutureRecord::to(topic).payload(&payload).key(key);
        self.producer
            .send(record, Duration::from_millis(self.config.timeout_ms))
            .await
            .map_err(|(e, _)| TopicEventBusError::Publish(e.to_string()))?;
        debug!(topic = %topic, event_id = %envelope.event_id, "EventBus published");
        Ok(())
    }

    fn subscribe(
        &self,
        topic: &str,
        consumer_group: &str,
    ) -> std::pin::Pin<Box<dyn Stream<Item = Result<EventEnvelope, TopicEventBusError>> + Send>> {
        let consumer = match Self::create_consumer(&self.config, consumer_group) {
            Ok(c) => c,
            Err(e) => return Box::pin(futures::stream::iter(vec![Err(e)])),
        };
        if let Err(e) = consumer.subscribe(&[topic]) {
            error!(error = %e, "Kafka subscribe failed");
            return Box::pin(futures::stream::iter(vec![Err(TopicEventBusError::Subscribe(
                e.to_string(),
            ))]));
        }
        let auto_commit = self.config.enable_auto_commit;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            loop {
                match consumer.recv().await {
                    Ok(borrowed) => {
                        if let Some(envelope) = decode_message(&borrowed) {
                            if !auto_commit {
                                if let Err(e) = consumer.commit_message(&borrowed, CommitMode::Async) {
                                    warn!(error = %e, "commit failed");
                                }
                            }
                            if tx.send(Ok(envelope)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Kafka recv error");
                        let _ = tx.send(Err(TopicEventBusError::Subscribe(e.to_string()))).await;
                        break;
                    }
                }
            }
        });
        Box::pin(ReceiverStream::new(rx))
    }
}

fn decode_message(msg: &BorrowedMessage<'_>) -> Option<EventEnvelope> {
    let payload = msg.payload()?;
    serde_json::from_slice(payload).ok()
}
