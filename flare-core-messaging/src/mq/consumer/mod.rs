//! MQ 消费者框架
//!
//! 提供统一的消费处理框架，只包含抽象层级和处理层。
//!
//! ## 架构设计
//!
//! - **Handler**: 统一的消息处理器接口
//! - **Dispatcher**: 按 topic 路由消息到对应 Handler
//! - **Runtime**: 消费循环和并发控制
//! - **Fetcher**: 消息获取器抽象接口
//!
//! 注意：具体的 MQ 实现（如 Kafka、NATS、RocketMQ）应该在各自的模块中实现。

pub mod adapter;
pub mod dispatcher;
pub mod handler;
pub mod runtime;
pub mod task;
pub mod types;

// 重新导出核心类型
pub use adapter::{MqConsumerAdapter, MqConsumerAdapterBuilder};
pub use dispatcher::{Dispatcher, RegistryDispatcher, TopicDispatcher};
pub use handler::{HandlerRegistry, MessageHandler};
pub use runtime::{
    ConsumerConfig, ConsumerRuntime, ConsumerRuntimeTask, ConsumerStats, MessageFetcher,
};
pub use task::{MqConsumer, MqConsumerTask};
pub use types::{ConsumerError, ContentType, Message, MessageContext, MessageResult};
