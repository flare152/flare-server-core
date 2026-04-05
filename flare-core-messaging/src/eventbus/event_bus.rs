//! 统一事件总线（Event Bus）契约
//!
//! 提供订阅和发布的统一接口，支持内存和 MQ 两种实现
//!
//! ## 架构
//! - **EventPublisher**: 事件发布接口
//! - **EventSubscriber**: 事件订阅接口
//! - **EventHandler**: 事件处理器接口
//!
//! ## 不变量
//! - **顺序**：跨进程有序性由 Broker 对 `partition_key` 的保证 + 业务 `seq` 共同约束
//! - **上下文**：`Ctx` 由调用方传入；MQ 实现写入头，内存实现进入 TopicBroadcast

use std::sync::Arc;

use async_trait::async_trait;

use flare_core_base::context::Ctx;
use flare_core_base::error::Result;

use super::envelope::EventEnvelope;

// =============================================================================
// EventPublisher - 事件发布接口
// =============================================================================

/// 事件发布者接口
///
/// 支持发布单个事件或批量发布事件
pub trait EventPublisher: Send + Sync {
    /// 发布单个事件
    ///
    /// # 参数
    /// - `ctx`: 上下文信息，包含追踪 ID、用户信息等
    /// - `topic`: 事件主题
    /// - `envelope`: 事件信封
    ///
    /// # 返回
    /// - `Result<()>`: 发布成功或失败
    fn publish<'a>(
        &'a self,
        ctx: &'a Ctx,
        topic: &'a str,
        envelope: &'a EventEnvelope,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a;

    /// 批量发布事件（默认顺序调用 `publish`，保证同一 topic 下发送次序与调用顺序一致）
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

/// 对 `Arc<P>` 委托实现，便于 `T: EventPublisher` 泛型与组合根直接持有 `Arc<impl EventPublisher>`
impl<P> EventPublisher for Arc<P>
where
    P: EventPublisher + ?Sized,
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

// =============================================================================
// EventSubscriber - 事件订阅接口
// =============================================================================

/// 事件订阅者接口
///
/// 负责订阅主题并处理接收到的事件
pub trait EventSubscriber: Send + Sync {
    /// 订阅主题并开始消费事件
    ///
    /// # 参数
    /// - `topic`: 要订阅的主题
    /// - `handler`: 事件处理器
    ///
    /// # 返回
    /// - `Result<()>`: 订阅成功或失败
    fn subscribe<'a>(
        &'a self,
        topic: &'a str,
        handler: Arc<dyn EventHandler>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a;

    /// 订阅主题并指定消费者组（Kafka 等为 `group.id`；内存等实现可忽略 `consumer_group`）
    fn subscribe_with_consumer_group<'a>(
        &'a self,
        topic: &'a str,
        consumer_group: Option<&'a str>,
        handler: Arc<dyn EventHandler>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        async move {
            let _ = consumer_group;
            self.subscribe(topic, handler).await
        }
    }

    /// 停止订阅
    ///
    /// # 返回
    /// - `Result<()>`: 停止成功或失败
    fn unsubscribe<'a>(
        &'a self,
        _topic: &'a str,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        async move {
            // 默认实现：什么都不做
            Ok(())
        }
    }
}

/// 对 `Arc<P>` 委托实现，便于 `T: EventSubscriber` 泛型与组合根直接持有 `Arc<impl EventSubscriber>`
impl<P> EventSubscriber for Arc<P>
where
    P: EventSubscriber + ?Sized,
{
    fn subscribe<'a>(
        &'a self,
        topic: &'a str,
        handler: Arc<dyn EventHandler>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        async move { (**self).subscribe(topic, handler).await }
    }

    fn subscribe_with_consumer_group<'a>(
        &'a self,
        topic: &'a str,
        consumer_group: Option<&'a str>,
        handler: Arc<dyn EventHandler>,
    ) -> impl std::future::Future<Output = Result<()>> + Send + 'a {
        async move {
            (**self)
                .subscribe_with_consumer_group(topic, consumer_group, handler)
                .await
        }
    }
}

// =============================================================================
// EventHandler - 事件处理器接口
// =============================================================================

/// 事件处理器接口
///
/// 处理接收到的事件，`ctx` 来自消息上下文
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// 处理事件
    ///
    /// # 参数
    /// - `ctx`: 上下文信息，从消息头中提取
    /// - `envelope`: 事件信封
    ///
    /// # 返回
    /// - `Result<()>`: 处理成功或失败
    async fn handle(&self, ctx: &Ctx, envelope: EventEnvelope) -> Result<()>;

    /// 获取处理器名称
    ///
    /// # 返回
    /// - `&str`: 处理器名称
    fn name(&self) -> &str;
}

// =============================================================================
// EventBus - 统一事件总线接口
// =============================================================================

/// 统一事件总线接口
///
/// 组合了发布和订阅功能
pub trait EventBus: EventPublisher + EventSubscriber {}

// 对 `Arc<P>` 委托实现
impl<P> EventBus for Arc<P> where P: EventBus + ?Sized {}
