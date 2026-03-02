//! 基于 Topic 的分布式事件总线抽象（Kafka / NATS 等）

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

use super::envelope::EventEnvelope;

/// 基于 Topic 的事件总线：支持多种后端（Kafka / NATS）
#[async_trait]
pub trait TopicEventBus: Send + Sync {
    /// 发布单条事件到指定 topic；按 envelope.partition_key() 分区
    async fn publish(&self, topic: &str, envelope: &EventEnvelope) -> Result<(), TopicEventBusError>;

    /// 批量发布事件，默认实现为逐条 publish；实现可覆盖以优化吞吐
    async fn publish_batch(
        &self,
        topic: &str,
        envelopes: &[EventEnvelope],
    ) -> Result<(), TopicEventBusError> {
        for envelope in envelopes {
            self.publish(topic, envelope).await?;
        }
        Ok(())
    }

    /// 订阅 topic，使用 consumer_group 实现 at-least-once
    fn subscribe(
        &self,
        topic: &str,
        consumer_group: &str,
    ) -> Pin<Box<dyn Stream<Item = Result<EventEnvelope, TopicEventBusError>> + Send>>;
}

/// 事件总线错误
#[derive(Debug, thiserror::Error)]
pub enum TopicEventBusError {
    #[error("publish failed: {0}")]
    Publish(String),
    #[error("subscribe/consumer failed: {0}")]
    Subscribe(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("deserialization failed: {0}")]
    Deserialization(String),
}
