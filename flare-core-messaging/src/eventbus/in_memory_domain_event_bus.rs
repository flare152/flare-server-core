//! 进程内领域事件总线实现（`tokio::sync::broadcast`）

use async_trait::async_trait;
use tokio::sync::broadcast;

use flare_core_base::error::{FlareError, Result};

use super::domain_event_bus::{DomainEvent, EventBus};

/// 内存版事件总线（单机 CQRS / 模块解耦）
pub struct InMemoryEventBus<E: DomainEvent> {
    sender: broadcast::Sender<E>,
}

impl<E: DomainEvent> InMemoryEventBus<E> {
    /// `capacity`：广播缓冲容量（慢消费者可能丢包，建议 1024～10000）
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
}

#[async_trait]
impl<E: DomainEvent> EventBus<E> for InMemoryEventBus<E> {
    async fn publish(&self, event: E) -> Result<()> {
        self.sender.send(event).map_err(|e| {
            FlareError::message_send_failed(format!("in-memory event bus broadcast: {e}"))
        })?;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<E> {
        self.sender.subscribe()
    }
}
