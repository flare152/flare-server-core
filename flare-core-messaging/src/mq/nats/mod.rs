//! NATS JetStream 实现模块
//!
//! 提供 NATS JetStream 的生产者和消费者实现，支持 Context 透传。

pub mod config;
pub mod consumer;
pub mod producer;

// 重新导出配置 Trait
pub use config::{NatsConsumerConfig, NatsProducerConfig};

// 重新导出 Producer 和 Consumer
pub use consumer::{NatsConsumerBuilder, NatsConsumerRuntime, NatsMessageFetcher};
pub use producer::{NatsProducer, NatsProducerBuilder};
