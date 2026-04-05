//! Kafka 实现模块
//!
//! 提供 Kafka 的生产者和消费者实现，支持 Context 透传。

pub mod config;
pub mod consumer;
pub mod producer;

// 重新导出配置 Trait
pub use config::{KafkaConsumerConfig, KafkaConsumerGroupOverride, KafkaProducerConfig};

// 重新导出构建函数
pub use config::{build_kafka_consumer, build_kafka_producer, subscribe_and_wait_for_assignment};

// 重新导出 Producer 和 Consumer
pub use consumer::{
    KafkaConsumerBuilder, KafkaConsumerRuntime, KafkaMessageFetcher, context_from_kafka_headers,
    message_from_kafka_borrowed,
};
pub use producer::{KafkaProducer, KafkaProducerBuilder};
