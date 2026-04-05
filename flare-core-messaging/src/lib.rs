//! Flare Core Messaging - 消息层
//!
//! 提供消息队列和事件总线能力

pub mod eventbus;
pub mod mq;

// Re-exports - MQ
pub use mq::{MessageHandler, MqConsumer, MqConsumerTask, Producer};

// Re-exports - Event Bus
pub use eventbus::{
    EventBus, EventEnvelope, EventSubscriber, InMemoryEventBus, InMemoryTopicEventBus,
    MqTopicEventBus, TopicEventBus,
};
