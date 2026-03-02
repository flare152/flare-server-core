//! 事件总线接口与实现

use async_trait::async_trait;
use anyhow::Result;
use tokio::sync::broadcast;

/// 领域事件 trait
/// 
/// 所有通过事件总线传递的事件必须实现此 trait
pub trait DomainEvent: Send + Sync + Clone {
    /// 事件类型标识（如 "session.created"）
    fn event_type(&self) -> &'static str;
    
    /// 序列化为 JSON（用于持久化或跨服务传输）
    fn to_json(&self) -> Result<String>;
}

/// 事件订阅者 trait
/// 
/// 实现此 trait 以处理特定类型的事件
#[async_trait]
pub trait EventSubscriber<E: DomainEvent>: Send + Sync {
    /// 处理接收到的事件
    async fn on_event(&self, event: E) -> Result<()>;
}

/// 事件总线接口
/// 
/// 定义事件发布/订阅的统一接口，支持不同实现（内存/Redis/Kafka）
#[async_trait]
pub trait EventBus<E: DomainEvent>: Send + Sync {
    /// 发布事件到总线
    async fn publish(&self, event: E) -> Result<()>;
    
    /// 订阅事件（返回接收器）
    fn subscribe(&self) -> broadcast::Receiver<E>;
}

/// 内存版事件总线（基于 tokio broadcast channel）
/// 
/// 适用场景：
/// - 单机部署或开发测试环境
/// - 进程内模块解耦通信
/// - CQRS 读写模型事件投影
/// 
/// 限制：
/// - 仅支持进程内通信，不支持跨服务
/// - 事件不持久化，进程重启后丢失
pub struct InMemoryEventBus<E: DomainEvent> {
    sender: broadcast::Sender<E>,
}

impl<E: DomainEvent> InMemoryEventBus<E> {
    /// 创建新的内存事件总线
    /// 
    /// # 参数
    /// - `capacity`: 事件通道容量（建议 1000-10000）
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
}

#[async_trait]
impl<E: DomainEvent> EventBus<E> for InMemoryEventBus<E> {
    async fn publish(&self, event: E) -> Result<()> {
        self.sender.send(event)
            .map_err(|e| anyhow::anyhow!("Failed to publish event: {}", e))?;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<E> {
        self.sender.subscribe()
    }
}
