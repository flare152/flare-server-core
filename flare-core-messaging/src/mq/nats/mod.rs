//! NATS JetStream 实现模块
//!
//! 提供 NATS JetStream 的生产者和消费者实现，支持 Context 透传。

pub mod config;
pub mod consumer;
pub mod producer;

// 重新导出配置 Trait
pub use config::{
    NatsConsumerConfig, NatsProducerConfig, NatsStreamSpec, default_stream_specs,
    resolve_subject_stream, subject_matches,
};

// 重新导出 Producer 和 Consumer
pub use consumer::{
    NatsConsumerBuilder, NatsConsumerRuntime, NatsMessageFetcher, build_nats_consumer_tasks,
    build_nats_consumer_tasks_with_failure_publishers,
};
pub use producer::{NatsProducer, NatsProducerBuilder};
