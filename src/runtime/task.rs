//! 任务定义模块
//!
//! 提供统一的任务抽象，支持消息消费者等不同类型的任务

use std::future::Future;
use std::pin::Pin;

/// 任务执行结果
pub type TaskResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// 任务 trait
///
/// 所有需要在运行时中管理的任务都必须实现此 trait
pub trait Task: Send {
    /// 获取任务名称
    fn name(&self) -> &str;
    
    /// 获取任务依赖
    ///
    /// 返回此任务依赖的其他任务名称列表
    /// 依赖的任务会在此任务之前启动
    /// 默认实现返回空列表（无依赖）
    fn dependencies(&self) -> Vec<String> {
        Vec::new()
    }
    
    /// 运行任务
    ///
    /// # 参数
    /// * `shutdown_rx` - 关闭信号接收器，当收到信号时任务应该优雅关闭
    fn run(
        self: Box<Self>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>>;
    
    /// 就绪检查
    ///
    /// 用于检查任务是否已经就绪（例如 gRPC 服务是否已经可以接受连接）
    /// 默认实现总是返回成功
    fn ready_check(
        &self,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<(), Box<dyn std::error::Error + Send + Sync>>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(()) })
    }
}

// -------- Message Consumer Task --------

/// 消息消费者 trait
///
/// 所有消息消费者（Kafka、RabbitMQ 等）都应该实现此 trait
pub trait MessageConsumer: Send + Sync {
    /// 消费消息
    ///
    /// # 参数
    /// * `shutdown_rx` - 关闭信号接收器
    fn consume(
        &self,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send + '_>>;
}

/// 消息消费者任务
///
/// 将实现了 `MessageConsumer` trait 的对象包装成 `Task`
pub struct MessageConsumerTask {
    name: String,
    dependencies: Vec<String>,
    consumer: Box<dyn MessageConsumer + Send + Sync>,
}

impl MessageConsumerTask {
    /// 创建新的消息消费者任务
    pub fn new(name: impl Into<String>, consumer: Box<dyn MessageConsumer + Send + Sync>) -> Self {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            consumer,
        }
    }
    
    /// 设置任务依赖
    ///
    /// # 参数
    /// * `deps` - 依赖的任务名称列表
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }
}

impl Task for MessageConsumerTask {
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

// -------- Spawn Task --------

/// Spawn 任务
///
/// 用于包装已经构建好的 Future（例如 gRPC server）
/// 用户可以在 service 层构建好 Future，然后通过 runtime 管理
///
/// 注意：如果 Future 需要 shutdown_rx，应该使用闭包延迟构建
pub struct SpawnTask {
    name: String,
    dependencies: Vec<String>,
    // 使用闭包延迟构建 Future，以便在 run 时传入 shutdown_rx
    future_fn: Box<dyn FnOnce(tokio::sync::oneshot::Receiver<()>) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> + Send + 'static>,
}

impl SpawnTask {
    /// 创建新的 spawn 任务（不需要 shutdown_rx）
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future` - 要运行的 Future（不依赖 shutdown_rx）
    pub fn new<Fut>(name: impl Into<String>, future: Fut) -> Self
    where
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            future_fn: Box::new(move |_shutdown_rx| Box::pin(future)),
        }
    }
    
    /// 创建新的 spawn 任务（需要 shutdown_rx）
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future_fn` - 闭包，接收 shutdown_rx，返回 Future
    ///
    /// # 示例
    /// ```rust,no_run
    /// use flare_server_core::runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::with_shutdown("my-task", |shutdown_rx| {
    ///     Box::pin(async move {
    ///         // 使用 shutdown_rx
    ///         let _ = shutdown_rx.await;
    ///         Ok(())
    ///     })
    /// });
    /// ```
    pub fn with_shutdown<F, Fut>(name: impl Into<String>, future_fn: F) -> Self
    where
        F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            future_fn: Box::new(move |shutdown_rx| Box::pin(future_fn(shutdown_rx))),
        }
    }
    
    /// 设置任务依赖
    ///
    /// # 参数
    /// * `deps` - 依赖的任务名称列表
    ///
    /// # 示例
    /// ```rust,no_run
    /// use flare_server_core::runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::new("task-b", async { Ok(()) })
    ///     .with_dependencies(vec!["task-a".to_string()]);
    /// ```
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }
}

impl Task for SpawnTask {
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
        // 调用 future_fn，传入 shutdown_rx
        (self.future_fn)(shutdown_rx)
    }
}

// 注意：CustomTask 已被 SpawnTask::with_shutdown 替代，保留以保持向后兼容
// 建议使用 SpawnTask::with_shutdown 或 runtime.add_spawn_with_shutdown
