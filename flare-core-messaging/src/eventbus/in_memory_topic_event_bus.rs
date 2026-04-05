//! 内存版 [super::topic_event_bus::TopicEventBus]：`tokio::sync::broadcast` 扇出，与 MQ 版 API 对齐（`publish` 传 `Ctx`）

use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::debug;

use flare_core_base::context::Ctx;
use flare_core_base::error::{FlareError, Result};

use super::envelope::EventEnvelope;
use super::topic_event_bus::TopicEventBus;

/// 默认广播槽位（慢消费者易 `Lagged`，生产可酌情调大）
pub const DEFAULT_TOPIC_BROADCAST_CAPACITY: usize = 1024;

/// 一次 Topic 发布的快照（订阅方 `recv` 得到；可按 `topic` 过滤）
#[derive(Clone, Debug)]
pub struct TopicBroadcast {
    pub topic: Arc<str>,
    pub ctx: Ctx,
    pub envelope: EventEnvelope,
}

/// 内存 Topic 总线：同进程多订阅者；**不**跨进程、不持久化
pub struct InMemoryTopicEventBus {
    tx: broadcast::Sender<TopicBroadcast>,
}

impl InMemoryTopicEventBus {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _) = broadcast::channel(capacity);
        Arc::new(Self { tx })
    }

    /// 使用 [DEFAULT_TOPIC_BROADCAST_CAPACITY]
    pub fn new_default() -> Arc<Self> {
        Self::new(DEFAULT_TOPIC_BROADCAST_CAPACITY)
    }

    /// 当前活跃订阅者数（调试用容量规划）
    #[inline]
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// 订阅所有 topic；业务侧在循环中过滤 `msg.topic`
    pub fn subscribe(&self) -> broadcast::Receiver<TopicBroadcast> {
        self.tx.subscribe()
    }
}

impl TopicEventBus for InMemoryTopicEventBus {
    async fn publish(&self, ctx: &Ctx, topic: &str, envelope: &EventEnvelope) -> Result<()> {
        let msg = TopicBroadcast {
            topic: Arc::from(topic),
            ctx: ctx.clone(),
            envelope: envelope.clone(),
        };
        self.tx.send(msg).map_err(|e| {
            FlareError::message_send_failed(format!("in-memory topic bus broadcast: {e}"))
        })?;
        debug!(topic = %topic, event_id = %envelope.event_id, "Topic event published (in-memory)");
        Ok(())
    }
}
