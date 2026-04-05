//! 统一的消息队列抽象层
//!
//! 提供通用的 MQ 接口和 Kafka、NATS 实现，供各个业务模块使用。
//!
//! ## 功能特性
//!
//! - **通用配置接口**: `KafkaProducerConfig`、`KafkaConsumerConfig`、`NatsProducerConfig`、`NatsConsumerConfig` trait
//! - **便捷构建器**: `build_kafka_producer`、`build_kafka_consumer` 函数
//! - **消费者框架**: 统一的 Handler 模型、并发控制
//! - **生产者框架**: 统一的 Producer 接口、批量发送
//! - **Context 透传支持**: 自动处理上下文信息的编码和解码
//! - **多 MQ 支持**: Kafka 和 NATS JetStream
//!
//! ## 架构设计
//!
//! - `producer.rs`: 生产者核心框架（抽象层）
//! - `consumer/`: 消费者核心框架（抽象层）
//! - `kafka/`: Kafka 具体实现（包含 Context 透传）
//! - `nats/`: NATS JetStream 具体实现（包含 Context 透传）
//!
//! ## 使用示例
//!
//! ### 生产者
//!
//! ```rust,no_run
//! use flare_server_core::mq::producer::{Producer, ProducerMessage, ProducerConfig};
//! use flare_server_core::mq::kafka::{KafkaProducerBuilder, KafkaProducerConfig};
//!
//! struct MyConfig;
//!
//! impl KafkaProducerConfig for MyConfig {
//!     fn kafka_bootstrap(&self) -> &str {
//!         "localhost:9092"
//!     }
//!     // ... 其他配置方法
//! }
//!
//! let builder = KafkaProducerBuilder::new()
//!     .with_config(ProducerConfig::default());
//!
//! let producer = builder.build(&MyConfig)?;
//!
//! // 创建 Context
//! let ctx = Context::with_request_id("req-123".to_string());
//!
//! producer.send(
//!     &ctx,
//!     "test.topic",
//!     Some("key123"),
//!     b"Hello, Kafka!".to_vec(),
//!     None,
//! ).await?;
//! ```
//!
//! ### 消费者
//!
//! ```rust,no_run
//! use flare_server_core::mq::consumer::{
//!     KafkaConsumerBuilder, ConsumerConfig, MessageHandler, Message,
//!     MessageResult, ConsumerError,
//! };
//!
//! struct MyHandler;
//!
//! #[async_trait::async_trait]
//! impl MessageHandler for MyHandler {
//!     async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
//!         let ctx = &message.context;
//!         tracing::info!("Processing message: trace_id = {}", ctx.ctx.trace_id());
//!         Ok(MessageResult::Ack)
//!     }
//!
//!     fn name(&self) -> &str {
//!         "my_handler"
//!     }
//! }
//!
//! let builder = KafkaConsumerBuilder::new()
//!     .register_handler(MyHandler, vec!["test.topic".to_string()])?;
//!
//! let runtime = builder.build();
//! runtime.start(kafka_config).await?;
//! ```

#[cfg(feature = "kafka")]
pub mod kafka;

#[cfg(feature = "nats")]
pub mod nats;

pub mod consumer;
pub mod context;
pub mod producer;

// 重新导出 Producer 相关类型
pub use producer::{Producer, ProducerConfig, ProducerError, ProducerMessage};

// 重新导出 Consumer 相关类型
pub use consumer::{
    ConsumerConfig, ConsumerError, ConsumerRuntime, ConsumerRuntimeTask, ConsumerStats,
    ContentType, Dispatcher, HandlerRegistry, Message, MessageContext, MessageFetcher,
    MessageHandler, MessageResult, MqConsumer, MqConsumerTask, RegistryDispatcher, TopicDispatcher,
};

#[cfg(feature = "kafka")]
pub use kafka::{
    KafkaConsumerBuilder, KafkaConsumerConfig, KafkaConsumerRuntime, KafkaMessageFetcher,
    KafkaProducer, KafkaProducerBuilder, KafkaProducerConfig, build_kafka_consumer,
    build_kafka_producer, subscribe_and_wait_for_assignment,
};

#[cfg(feature = "nats")]
pub use nats::{
    NatsConsumerBuilder, NatsConsumerConfig, NatsConsumerRuntime, NatsMessageFetcher, NatsProducer,
    NatsProducerBuilder, NatsProducerConfig,
};
