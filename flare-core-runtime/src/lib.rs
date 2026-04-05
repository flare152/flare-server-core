//! Flare Core Runtime - 统一运行时框架
//!
//! 提供强大、稳定、通用的运行时系统，能够管理各类服务组件的生命周期
//!
//! # 核心特性
//!
//! - **统一服务启动**: HTTP (axum/volo-http/actix-web), gRPC (tonic/volo), MQ 消费者, 自定义任务
//! - **优雅停止**: 多信号源支持、依赖顺序关闭、超时强制终止
//! - **服务编排**: 任务依赖管理、健康检查、服务注册/注销
//! - **定时任务**: Cron 表达式调度、一次性延迟任务
//! - **状态监控**: 任务状态追踪、指标暴露、事件通知
//! - **可扩展性**: 插件化架构、中间件链、自定义适配器
//!
//! # 架构设计
//!
//! `flare-core-runtime` 只提供规范（trait 定义、配置、核心抽象），具体实现由其他 crate 提供：
//! - `flare-core-transport` 实现 HTTP/gRPC 适配器
//! - `flare-core-messaging` 实现 MQ 消费者适配器
//!
//! # 示例
//!
//! ```rust,no_run
//! use flare_core_runtime::{ServiceRuntime, Task, TaskResult};
//! use std::pin::Pin;
//! use std::future::Future;
//!
//! // 定义自定义任务
//! struct MyTask {
//!     name: String,
//! }
//!
//! impl Task for MyTask {
//!     fn name(&self) -> &str {
//!         &self.name
//!     }
//!
//!     fn run(
//!         self: Box<Self>,
//!         shutdown_rx: tokio::sync::oneshot::Receiver<()>,
//!     ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
//!         Box::pin(async move {
//!             // 任务逻辑
//!             Ok(())
//!         })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // 创建运行时
//!     let runtime = ServiceRuntime::new("my-service")
//!         .add_task(Box::new(MyTask { name: "task-1".to_string() }));
//!
//!     // 运行
//!     runtime.run().await?;
//!     Ok(())
//! }
//! ```

// 模块声明
pub mod config;
pub mod error;
pub mod health;
pub mod metrics;
pub mod middleware;
pub mod plugin;
pub mod registry;
pub mod runtime;
pub mod signal;
pub mod state;
pub mod task;
pub mod utils;

// Re-exports
pub use config::RuntimeConfig;
pub use error::{
    HealthError, MetricsError, MiddlewareError, PluginError, RegistryError, RuntimeError,
};
pub use health::{HealthCheck, HealthCheckResult, HealthChecker};
pub use metrics::MetricsCollector;
pub use middleware::{Middleware, MiddlewareChain};
pub use plugin::{Plugin, PluginContext, PluginManager};
pub use registry::{ServiceInfo, ServiceRegistry};
pub use runtime::{HealthFailureAction, ServiceRuntime};
pub use signal::{
    ChannelSignal, CompositeSignal, CtrlCSignal, ShutdownSignal, UnixSignal, UnixSignalKind,
};
pub use state::{RuntimeEvent, StateEvent, StateTracker, TaskStateInfo};
pub use task::{SpawnTask, Task, TaskManager, TaskResult, TaskState};
pub use utils::topological_sort;
