//! MQ 上消费 Topic 事件（如 [super::EventEnvelope]）时的任务类型
//!
//! 与 [crate::mq::consumer::MqConsumer] 契约相同，命名强调「事件总线 + MQ Topic」场景；实现上直接复用 `mq::consumer` 中的定义。

pub use crate::mq::consumer::{
    MqConsumer as TopicEventMqConsumer, MqConsumerTask as TopicEventMqConsumerTask,
};
