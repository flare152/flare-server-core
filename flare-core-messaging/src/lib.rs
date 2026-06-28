//! Event-bus and MQ primitives for Flare server applications.
//!
//! `flare-core-messaging` provides in-process event buses, topic-envelope
//! handling, MQ producer/consumer traits, and optional NATS or Kafka adapters.
//! Application services can use the same event contract in memory and across MQ
//! backends.

pub mod eventbus;
pub mod mq;

// MQ re-exports.
pub use mq::{MessageHandler, MqConsumer, MqConsumerTask, Producer};

// Event-bus re-exports.
pub use eventbus::{
    EventBus, EventEnvelope, EventSubscriber, InMemoryEventBus, InMemoryTopicEventBus,
    MqTopicEventBus, TopicEventBus,
};
