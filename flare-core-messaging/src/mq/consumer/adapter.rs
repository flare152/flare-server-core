//! MQ 消费者适配器
//!
//! 实现 Task trait,使 MQ 消费者可以被 ServiceRuntime 管理

use flare_core_runtime::task::{Task, TaskResult};
use std::future::Future;
use std::pin::Pin;

/// MQ 消费者适配器
///
/// 包装 MQ 消费者,实现 Task trait
///
/// # 示例
///
/// ```rust
/// use flare_core_messaging::mq::consumer::MqConsumerAdapter;
///
/// let adapter = MqConsumerAdapter::new("kafka-consumer", || async {
///     // 消费消息
///     Ok(())
/// });
/// ```
pub struct MqConsumerAdapter<F, Fut>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    name: String,
    dependencies: Vec<String>,
    critical: bool,
    consume_fn: Option<F>,
    _marker: std::marker::PhantomData<Fut>,
}

impl<F, Fut> MqConsumerAdapter<F, Fut>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    /// 创建新的 MQ 消费者适配器
    ///
    /// # 参数
    ///
    /// * `name` - 消费者名称
    /// * `consume_fn` - 消费函数
    pub fn new(name: impl Into<String>, consume_fn: F) -> Self {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            critical: false, // MQ 消费者默认为非关键任务
            consume_fn: Some(consume_fn),
            _marker: std::marker::PhantomData,
        }
    }

    /// 设置依赖
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    /// 设置是否为关键任务
    pub fn with_critical(mut self, critical: bool) -> Self {
        self.critical = critical;
        self
    }
}

impl<F, Fut> Task for MqConsumerAdapter<F, Fut>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn dependencies(&self) -> Vec<String> {
        self.dependencies.clone()
    }

    fn run(
        self: Box<Self>,
        _shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
        if let Some(consume_fn) = self.consume_fn {
            // MQ 消费者通常自己管理 shutdown,所以这里不传递 shutdown_rx
            Box::pin(consume_fn())
        } else {
            Box::pin(async { Err("consume_fn already consumed".into()) })
        }
    }

    fn is_critical(&self) -> bool {
        self.critical
    }
}

/// MQ 消费者适配器构建器
pub struct MqConsumerAdapterBuilder {
    name: String,
    dependencies: Vec<String>,
    critical: bool,
}

impl MqConsumerAdapterBuilder {
    /// 创建新的构建器
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            critical: false,
        }
    }

    /// 设置依赖
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    /// 设置是否为关键任务
    pub fn with_critical(mut self, critical: bool) -> Self {
        self.critical = critical;
        self
    }

    /// 构建 MQ 消费者适配器
    pub fn build<F, Fut>(self, consume_fn: F) -> MqConsumerAdapter<F, Fut>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
    {
        MqConsumerAdapter {
            name: self.name,
            dependencies: self.dependencies,
            critical: self.critical,
            consume_fn: Some(consume_fn),
            _marker: std::marker::PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mq_consumer_adapter_new() {
        let adapter = MqConsumerAdapter::new("test-consumer", || async { Ok(()) });

        assert_eq!(adapter.name(), "test-consumer");
        assert!(!adapter.is_critical()); // 默认非关键任务
    }

    #[test]
    fn test_mq_consumer_adapter_builder() {
        let adapter = MqConsumerAdapterBuilder::new("test-consumer")
            .with_dependencies(vec!["db".to_string()])
            .with_critical(true)
            .build(|| async { Ok(()) });

        assert_eq!(adapter.name(), "test-consumer");
        assert_eq!(adapter.dependencies(), vec!["db"]);
        assert!(adapter.is_critical());
    }
}
