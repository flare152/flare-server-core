//! SpawnTask 实现
//!
//! 用于包装已经构建好的 Future，是最基础的任务实现

use super::{Task, TaskResult};
use std::future::Future;
use std::pin::Pin;

/// Spawn 任务
///
/// 用于包装已经构建好的 Future（例如 gRPC server）
/// 用户可以在 service 层构建好 Future，然后通过 runtime 管理
///
/// # 特性
///
/// - 支持依赖声明
/// - 支持 shutdown 信号
/// - 支持优先级设置
/// - 支持关键任务标记
///
/// # 示例
///
/// ## 不需要 shutdown 的任务
///
/// ```rust
/// use flare_core_runtime::task::SpawnTask;
///
/// let task = SpawnTask::new("my-task", async {
///     // 任务逻辑
///     Ok(())
/// });
/// ```
///
/// ## 需要 shutdown 的任务
///
/// ```rust
/// use flare_core_runtime::task::SpawnTask;
///
/// let task = SpawnTask::with_shutdown("my-grpc", |shutdown_rx| {
///     async move {
///         // 使用 shutdown_rx 实现优雅停机
///         let _ = shutdown_rx.await;
///         Ok(())
///     }
/// });
/// ```
///
/// ## 带依赖的任务
///
/// ```rust
/// use flare_core_runtime::task::SpawnTask;
///
/// let task = SpawnTask::new("task-b", async { Ok(()) })
///     .with_dependencies(vec!["task-a".to_string()])
///     .with_priority(10)
///     .with_critical(true);
/// ```
pub struct SpawnTask {
    /// 任务名称
    name: String,
    /// 任务依赖
    dependencies: Vec<String>,
    /// 任务优先级
    priority: i32,
    /// 是否关键任务
    critical: bool,
    /// Future 构建函数
    ///
    /// 使用闭包延迟构建 Future，以便在 run 时传入 shutdown_rx
    future_fn: Box<
        dyn FnOnce(
                tokio::sync::oneshot::Receiver<()>,
            ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>>
            + Send
            + 'static,
    >,
}

impl SpawnTask {
    /// 创建新的 spawn 任务（不需要 shutdown_rx）
    ///
    /// # 参数
    ///
    /// * `name` - 任务名称
    /// * `future` - 要运行的 Future（不依赖 shutdown_rx）
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::new("my-task", async {
    ///     // 任务逻辑
    ///     Ok(())
    /// });
    /// ```
    pub fn new<Fut>(name: impl Into<String>, future: Fut) -> Self
    where
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        Self {
            name: name.into(),
            dependencies: Vec::new(),
            priority: 0,
            critical: false,
            future_fn: Box::new(move |_shutdown_rx| Box::pin(future)),
        }
    }

    /// 创建新的 spawn 任务（需要 shutdown_rx）
    ///
    /// # 参数
    ///
    /// * `name` - 任务名称
    /// * `future_fn` - 闭包，接收 shutdown_rx，返回 Future
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::with_shutdown("my-task", |shutdown_rx| {
    ///     async move {
    ///         // 使用 shutdown_rx
    ///         let _ = shutdown_rx.await;
    ///         Ok(())
    ///     }
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
            priority: 0,
            critical: false,
            future_fn: Box::new(move |shutdown_rx| Box::pin(future_fn(shutdown_rx))),
        }
    }

    /// 设置任务依赖
    ///
    /// # 参数
    ///
    /// * `deps` - 依赖的任务名称列表
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::new("task-b", async { Ok(()) })
    ///     .with_dependencies(vec!["task-a".to_string()]);
    /// ```
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    /// 设置任务优先级
    ///
    /// # 参数
    ///
    /// * `priority` - 优先级（数值越大优先级越高）
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::new("my-task", async { Ok(()) })
    ///     .with_priority(10);
    /// ```
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// 设置是否为关键任务
    ///
    /// # 参数
    ///
    /// * `critical` - 是否为关键任务
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::task::SpawnTask;
    ///
    /// let task = SpawnTask::new("my-task", async { Ok(()) })
    ///     .with_critical(true);
    /// ```
    pub fn with_critical(mut self, critical: bool) -> Self {
        self.critical = critical;
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

    fn priority(&self) -> i32 {
        self.priority
    }

    fn is_critical(&self) -> bool {
        self.critical
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn test_spawn_task_new() {
        let task = SpawnTask::new("test-task", async { Ok(()) });
        assert_eq!(task.name(), "test-task");
        assert!(task.dependencies().is_empty());
        assert_eq!(task.priority(), 0);
        assert!(!task.is_critical());
    }

    #[tokio::test]
    async fn test_spawn_task_with_dependencies() {
        let task = SpawnTask::new("test-task", async { Ok(()) })
            .with_dependencies(vec!["dep-1".to_string(), "dep-2".to_string()]);

        assert_eq!(task.dependencies(), vec!["dep-1", "dep-2"]);
    }

    #[tokio::test]
    async fn test_spawn_task_with_priority() {
        let task = SpawnTask::new("test-task", async { Ok(()) }).with_priority(10);

        assert_eq!(task.priority(), 10);
    }

    #[tokio::test]
    async fn test_spawn_task_with_critical() {
        let task = SpawnTask::new("test-task", async { Ok(()) }).with_critical(true);

        assert!(task.is_critical());
    }

    #[tokio::test]
    async fn test_spawn_task_run() {
        let task = SpawnTask::new("test-task", async { Ok(()) });
        let (_tx, rx) = oneshot::channel();

        let result = Box::new(task).run(rx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_spawn_task_with_shutdown_run() {
        let task = SpawnTask::with_shutdown("test-task", |shutdown_rx| {
            async move {
                // 模拟等待 shutdown 信号
                tokio::time::timeout(std::time::Duration::from_millis(100), shutdown_rx)
                    .await
                    .ok();
                Ok(())
            }
        });

        let (tx, rx) = oneshot::channel();

        // 发送 shutdown 信号
        tx.send(()).unwrap();

        let result = Box::new(task).run(rx).await;
        assert!(result.is_ok());
    }
}
