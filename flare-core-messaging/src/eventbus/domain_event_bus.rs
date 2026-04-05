//! 进程内领域事件：trait 定义（实现见 `in_memory_domain_event_bus`）

use async_trait::async_trait;
use tokio::sync::broadcast;

use flare_core_base::error::Result;

/// 领域事件 trait（进程内总线）
pub trait DomainEvent: Send + Sync + Clone {
    /// 事件类型标识（如 "session.created"）
    fn event_type(&self) -> &'static str;

    /// 序列化为 JSON（用于持久化或跨模块传输）
    fn to_json(&self) -> Result<String>;
}

/// 事件订阅者
#[async_trait]
pub trait EventSubscriber<E: DomainEvent>: Send + Sync {
    async fn on_event(&self, event: E) -> Result<()>;
}

/// 进程内事件总线接口
#[async_trait]
pub trait EventBus<E: DomainEvent>: Send + Sync {
    async fn publish(&self, event: E) -> Result<()>;

    fn subscribe(&self) -> broadcast::Receiver<E>;
}
