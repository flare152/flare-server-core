//! 事件总线（Event Bus）
//!
//! ## 进程内事件总线（内存）
//! - `DomainEvent` / `EventBus<E>` / `InMemoryEventBus`：基于 broadcast 的进程内事件传播
//!
//! ## 分布式事件总线（Topic）
//! - **核心抽象**（默认导出）：`EventEnvelope`、`TopicEventBus`、`TopicEventBusError`，不依赖 Kafka
//! - **Kafka 传输与后端**（feature `kafka`）：`kafka::` 通用生产者/消费者构建；`backend::kafka` 实现 TopicEventBus

mod event_bus;
pub mod envelope;
pub mod topic_bus;

#[cfg(feature = "kafka")]
pub mod kafka;
#[cfg(feature = "kafka")]
pub mod backend;

pub use event_bus::{DomainEvent, EventBus, EventSubscriber, InMemoryEventBus};
pub use envelope::EventEnvelope;
pub use topic_bus::{TopicEventBus, TopicEventBusError};

#[cfg(feature = "kafka")]
pub use backend::{KafkaEventBusConfig, KafkaTopicEventBus};
