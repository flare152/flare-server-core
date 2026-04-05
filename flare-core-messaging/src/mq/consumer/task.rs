//! MQ 消费者与 [crate::runtime::task::Task] 的桥接
//!
//! 通用「拉取型」MQ 消费者实现 [MqConsumer]，经 [MqConsumerTask] 注册到 [crate::runtime::ServiceRuntime]。

use std::future::Future;
use std::pin::Pin;

use flare_core_runtime::task::{Task, TaskResult};

/// 在 [crate::runtime::ServiceRuntime] 中运行的 MQ 消费者契约（Kafka / NATS 等）
pub trait MqConsumer: Send + Sync {
    /// 在收到 `shutdown_rx` 时应退出拉取循环并收尾
    fn consume(
        &self,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send + '_>>;
}

/// 将 [MqConsumer] 包装为 [Task]，供 [crate::runtime::ServiceRuntime::add_task] / [crate::runtime::ServiceRuntime::add_mq_consumer] 使用
pub struct MqConsumerTask {
    name: String,
    dependencies: Vec<String>,
    consumer: Box<dyn MqConsumer + Send + Sync>,
}

impl MqConsumerTask {
    pub fn new(name: impl Into<String>, consumer: Box<dyn MqConsumer + Send + Sync>) -> Self {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            consumer,
        }
    }

    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }
}

impl Task for MqConsumerTask {
    fn name(&self) -> &str {
        &self.name
    }

    fn dependencies(&self) -> Vec<String> {
        self.dependencies.clone()
    }

    fn run(
        self: Box<Self>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
        Box::pin(async move { self.consumer.consume(shutdown_rx).await })
    }
}
