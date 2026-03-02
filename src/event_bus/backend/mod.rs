//! 分布式事件总线后端实现
//!
//! 与 `discovery/backend`、`kv/backend` 一致：抽象在 event_bus 根，实现按后端分模块。

#[cfg(feature = "kafka")]
pub mod kafka;

#[cfg(feature = "kafka")]
pub use kafka::{KafkaEventBusConfig, KafkaTopicEventBus};
