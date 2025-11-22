//! Kafka 工具模块
//!
//! 提供通用的 Kafka 消费者和生产者构建工具
//!
//! 此模块需要启用 `kafka` feature 才能使用

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

