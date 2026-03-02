//! Kafka 传输层：通用生产者/消费者构建与事件总线 Kafka 后端共用
//!
//! 启用 `kafka` feature 时可用。事件总线 Kafka 实现（TopicEventBus）位于 `backend::kafka`。

#[cfg(feature = "kafka")]
pub mod consumer_config;
#[cfg(feature = "kafka")]
pub mod consumer_builder;
#[cfg(feature = "kafka")]
pub mod producer_config;
#[cfg(feature = "kafka")]
pub mod producer_builder;

#[cfg(feature = "kafka")]
pub use consumer_config::KafkaConsumerConfig;
#[cfg(feature = "kafka")]
pub use consumer_builder::{build_kafka_consumer, subscribe_and_wait_for_assignment};
#[cfg(feature = "kafka")]
pub use producer_config::KafkaProducerConfig;
#[cfg(feature = "kafka")]
pub use producer_builder::build_kafka_producer;
