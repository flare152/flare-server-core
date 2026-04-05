//! Task trait 定义
//!
//! 任务抽象是运行时的核心扩展点，所有需要在运行时中管理的任务都必须实现此 trait

use super::state::TaskState;
use std::future::Future;
use std::pin::Pin;

/// 任务执行结果
pub type TaskResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// 任务抽象 (核心扩展点)
///
/// 所有需要在运行时中管理的任务都必须实现此 trait
/// 支持依赖声明、生命周期钩子、就绪检查
///
/// # 实现说明
///
/// - 使用 Rust 2024 原生 async fn in traits（不使用 async-trait 宏）
/// - 所有方法提供默认实现，降低实现难度
/// - 支持依赖管理、优先级、关键任务标记
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::task::{Task, TaskResult};
/// use std::pin::Pin;
/// use std::future::Future;
///
/// struct MyTask {
///     name: String,
/// }
///
/// impl Task for MyTask {
///     fn name(&self) -> &str {
///         &self.name
///     }
///
///     fn run(
///         self: Box<Self>,
///         shutdown_rx: tokio::sync::oneshot::Receiver<()>,
///     ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
///         Box::pin(async move {
///             // 任务逻辑
///             Ok(())
///         })
///     }
/// }
/// ```
pub trait Task: Send {
    /// 获取任务名称
    ///
    /// 任务名称必须唯一，用于日志、指标和依赖管理
    fn name(&self) -> &str;

    /// 获取任务依赖
    ///
    /// 返回此任务依赖的其他任务名称列表
    /// 依赖的任务会在此任务之前启动
    ///
    /// # 默认实现
    ///
    /// 返回空列表（无依赖）
    fn dependencies(&self) -> Vec<String> {
        Vec::new()
    }

    /// 运行任务
    ///
    /// # 参数
    ///
    /// * `shutdown_rx` - 关闭信号接收器，当收到信号时任务应该优雅关闭
    ///
    /// # 返回
    ///
    /// 返回一个 Future，执行任务逻辑
    ///
    /// # 实现要求
    ///
    /// - 必须响应 shutdown_rx 信号，实现优雅停机
    /// - 错误时返回 `Err`，运行时会根据 `is_critical()` 决定是否停机
    fn run(
        self: Box<Self>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>>;

    /// 就绪检查 (可选)
    ///
    /// 用于检查任务是否已经就绪（例如 gRPC 服务是否已经可以接受连接）
    ///
    /// # 默认实现
    ///
    /// 总是返回成功
    fn ready_check(&self) -> Pin<Box<dyn Future<Output = TaskResult> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    /// 任务优先级 (用于启动顺序调整)
    ///
    /// 优先级高的任务先启动（在依赖关系满足的前提下）
    ///
    /// # 默认实现
    ///
    /// 返回 0（中等优先级）
    fn priority(&self) -> i32 {
        0
    }

    /// 是否关键任务 (失败时触发运行时停机)
    ///
    /// # 默认实现
    ///
    /// 返回 false（非关键任务）
    fn is_critical(&self) -> bool {
        false
    }

    /// 获取初始状态
    ///
    /// # 默认实现
    ///
    /// 返回 `TaskState::Pending`
    fn initial_state(&self) -> TaskState {
        TaskState::Pending
    }
}
