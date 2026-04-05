//! 事件总线（Event Bus）
//!
//! 说明见 **[README.md](README.md)**（进程内领域事件、内存/MQ 两种 `TopicEventBus`、`Ctx` 与 [crate::error::FlareError]）。

pub mod constants;
mod domain_event_bus;
pub mod envelope;
mod event_bus;
mod in_memory_domain_event_bus;
mod in_memory_topic_event_bus;
pub mod mq_consumer;
mod mq_topic;
mod topic_event_bus;

pub use constants::{EVENT_ENVELOPE_CONTENT_TYPE, HEADER_CONTENT_TYPE};
pub use domain_event_bus::{
    DomainEvent, EventBus as DomainEventBusTrait, EventSubscriber as DomainEventSubscriber,
};
pub use envelope::EventEnvelope;
pub use event_bus::{EventBus, EventHandler, EventPublisher, EventSubscriber};
pub use in_memory_domain_event_bus::InMemoryEventBus;
pub use in_memory_topic_event_bus::{
    DEFAULT_TOPIC_BROADCAST_CAPACITY, InMemoryTopicEventBus, TopicBroadcast,
};
pub use mq_consumer::{TopicEventMqConsumer, TopicEventMqConsumerTask};
#[allow(deprecated)]
pub use mq_topic::{
    MqEventBus,
    MqEventBusRuntime,
    MqEventHandler,
    TopicEnvelopeHandler,
    TopicEnvelopeMessageHandler,
    register_event_handler,
    register_topic_envelope_dispatcher,
    run_event_consumer,
    run_topic_event_consumer,
};
pub type MqTopicEventBus = MqEventBus;
pub use topic_event_bus::TopicEventBus;
