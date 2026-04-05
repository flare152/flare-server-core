//! Topic 事件总线契约：`TopicEventBus`（实现：`in_memory_topic_event_bus`、`mq_topic`）
//!
//! ## 不变量
//! - **顺序**：跨进程有序性由 Broker 对 `partition_key` 的保证 + 业务 `seq` 共同约束；`publish_batch` 默认**顺序**逐条发送，避免同 key 乱序。
//! - **上下文**：`Ctx` 由调用方传入；MQ 实现写入头，内存实现进入 [super::TopicBroadcast]。

use std::sync::Arc;

use flare_core_base::context::Ctx;
use flare_core_base::error::Result;

use super::envelope::EventEnvelope;

/// 基于 Topic 的事件总线（内存或 MQ）；错误统一为 [crate::error::FlareError]
pub trait TopicEventBus: Send + Sync {
    /// 发布；分布式实现将 `ctx` 写入 MQ 头；内存实现扇出 [super::in_memory_topic_event_bus::TopicBroadcast]
    fn publish<'a>(
        &'a self,
        ctx: &'a Ctx,
        topic: &'a str,
        envelope: &'a EventEnvelope,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a;

    /// 批量发布（默认**顺序**调用 `publish`，保证同一 `topic` 下发送次序与调用顺序一致）
    fn publish_batch<'a>(
        &'a self,
        ctx: &'a Ctx,
        topic: &'a str,
        envelopes: &'a [EventEnvelope],
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        async move {
            for envelope in envelopes {
                self.publish(ctx, topic, envelope).await?;
            }
            Ok(())
        }
    }
}

/// 对 `Arc<P>` 委托实现，便于 `T: TopicEventBus` 泛型与组合根直接持有 `Arc<impl TopicEventBus>`
impl<P> TopicEventBus for Arc<P>
where
    P: TopicEventBus + ?Sized,
{
    fn publish<'a>(
        &'a self,
        ctx: &'a Ctx,
        topic: &'a str,
        envelope: &'a EventEnvelope,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        async move { (**self).publish(ctx, topic, envelope).await }
    }
}
