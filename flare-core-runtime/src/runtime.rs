//! ServiceRuntime 核心实现
//!
//! 提供统一的服务生命周期管理，支持：
//! - 多种服务类型 (HTTP, gRPC, MQ 消费者, 自定义任务)
//! - 优雅停机
//! - 服务注册/注销
//! - 健康检查
//! - 状态监控

use crate::config::RuntimeConfig;
use crate::error::HealthError;
use crate::health::{HealthCheck, HealthChecker};
use crate::registry::ServiceRegistry;
use crate::signal::{CompositeSignal, CtrlCSignal, ShutdownSignal, UnixSignal, UnixSignalKind};
use crate::state::StateTracker;
use crate::task::{SpawnTask, Task, TaskManager};
use anyhow::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// 默认健康检查：监控任务失败状态
struct TaskFailureHealthCheck {
    tracker: Arc<StateTracker>,
}

impl TaskFailureHealthCheck {
    fn new(tracker: Arc<StateTracker>) -> Self {
        Self { tracker }
    }
}

impl HealthCheck for TaskFailureHealthCheck {
    fn check(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = std::result::Result<(), HealthError>> + Send + '_>,
    > {
        Box::pin(async move {
            if self.tracker.has_failures().await {
                let failed = self.tracker.get_failed_tasks().await;
                return Err(HealthError::CheckFailed {
                    name: self.name().to_string(),
                    reason: format!("failed tasks detected: {:?}", failed),
                });
            }
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "task-failure-monitor"
    }
}

/// 健康检查失败后的运行时行为
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthFailureAction {
    /// 仅记录告警，不主动停机
    LogOnly,
    /// 触发优雅停机
    GracefulShutdown,
}

struct HealthMonitorHandle {
    stop_tx: oneshot::Sender<()>,
    join_handle: JoinHandle<()>,
    failure_rx: Option<mpsc::UnboundedReceiver<String>>,
}

/// 服务运行时
///
/// 统一管理服务的生命周期，包括：
/// - 任务启动和管理（HTTP, gRPC, MQ 消费者等）
/// - 服务注册和注销
/// - 优雅停机
/// - 状态监控
///
/// # 示例
///
/// ## 简单模式（不注册服务）
///
/// ```rust,no_run
/// use flare_core_runtime::ServiceRuntime;
/// use flare_core_runtime::task::SpawnTask;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let runtime = ServiceRuntime::new("my-service")
///         .add_spawn("my-task", async { Ok(()) });
///
///     runtime.run().await?;
///     Ok(())
/// }
/// ```
///
/// ## 完整模式（带服务注册）
///
/// ```rust,no_run
/// use flare_core_runtime::ServiceRuntime;
/// use std::net::SocketAddr;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let runtime = ServiceRuntime::new("my-service")
///         .with_address("0.0.0.0:8080".parse().unwrap())
///         .add_spawn("grpc", async { Ok(()) });
///
///     runtime.run_with_registration(|addr| {
///         Box::pin(async move {
///             // 注册服务
///             Ok(None)
///         })
///     }).await?;
///     Ok(())
/// }
/// ```
pub struct ServiceRuntime {
    /// 服务名称
    service_name: String,
    /// 服务地址（用于服务注册）
    service_address: Option<SocketAddr>,
    /// 任务管理器
    task_manager: TaskManager,
    /// 服务注册器
    registry: Option<Box<dyn ServiceRegistry>>,
    /// 配置
    config: RuntimeConfig,
    /// 健康检查器（可选）
    health_checker: Option<HealthChecker>,
    /// 健康检查失败动作
    health_failure_action: HealthFailureAction,
}

impl ServiceRuntime {
    /// 创建新的服务运行时
    ///
    /// # 参数
    ///
    /// * `service_name` - 服务名称（用于日志和服务注册）
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::ServiceRuntime;
    ///
    /// let runtime = ServiceRuntime::new("my-service");
    /// ```
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            service_address: None,
            task_manager: TaskManager::new(),
            registry: None,
            config: RuntimeConfig::default(),
            health_checker: None,
            health_failure_action: HealthFailureAction::LogOnly,
        }
    }

    /// 创建简单的任务运行器（无需服务名和地址）
    ///
    /// 用于运行 MQ 消费者、自定义任务等简单场景
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::ServiceRuntime;
    ///
    /// // 仅运行 MQ 消费者
    /// let runtime = ServiceRuntime::simple()
    ///     .add_spawn("kafka-consumer", async { Ok(()) });
    /// ```
    pub fn simple() -> Self {
        Self {
            service_name: "simple-runtime".to_string(),
            service_address: None,
            task_manager: TaskManager::new(),
            registry: None,
            config: RuntimeConfig::default(),
            health_checker: None,
            health_failure_action: HealthFailureAction::LogOnly,
        }
    }

    /// 创建 MQ 消费者运行器
    ///
    /// 专门用于运行 MQ 消费者的便捷方法
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::ServiceRuntime;
    ///
    /// let runtime = ServiceRuntime::mq_consumer()
    ///     .add_spawn("kafka-consumer", async { Ok(()) })
    ///     .add_spawn("nats-consumer", async { Ok(()) });
    /// ```
    pub fn mq_consumer() -> Self {
        Self::simple()
    }

    /// 创建自定义任务运行器
    ///
    /// 专门用于运行自定义任务的便捷方法
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::ServiceRuntime;
    ///
    /// let runtime = ServiceRuntime::tasks()
    ///     .add_spawn("task-1", async { Ok(()) })
    ///     .add_spawn("task-2", async { Ok(()) });
    /// ```
    pub fn tasks() -> Self {
        Self::simple()
    }

    /// 设置服务地址
    ///
    /// # 参数
    ///
    /// * `address` - 服务地址（用于服务注册）
    pub fn with_address(mut self, address: SocketAddr) -> Self {
        self.service_address = Some(address);
        self
    }

    /// 设置运行时配置
    pub fn with_config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }

    /// 设置服务注册器
    pub fn with_registry(mut self, registry: Box<dyn ServiceRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// 设置健康检查器
    pub fn with_health_checker(mut self, checker: HealthChecker) -> Self {
        self.health_checker = Some(checker);
        self
    }

    /// 添加健康检查项
    pub fn add_health_check(mut self, check: Arc<dyn HealthCheck>) -> Self {
        if let Some(checker) = &mut self.health_checker {
            checker.add_check(check);
        } else {
            let mut checker = HealthChecker::new()
                .with_failure_threshold(self.config.health_check.failure_threshold);
            checker.add_check(check);
            self.health_checker = Some(checker);
        }
        self
    }

    /// 设置健康检查失败时的行为
    pub fn with_health_failure_action(mut self, action: HealthFailureAction) -> Self {
        self.health_failure_action = action;
        self
    }

    /// 添加任务
    ///
    /// # 参数
    ///
    /// * `task` - 要添加的任务（实现了 `Task` trait）
    pub fn add_task(mut self, task: Box<dyn Task>) -> Self {
        self.task_manager.add_task(task);
        self
    }

    /// 添加 spawn 任务（直接添加 Future）
    ///
    /// # 参数
    ///
    /// * `name` - 任务名称
    /// * `future` - 要运行的 Future
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_core_runtime::ServiceRuntime;
    ///
    /// let runtime = ServiceRuntime::new("my-service")
    ///     .add_spawn("my-task", async { Ok(()) });
    /// ```
    pub fn add_spawn<Fut>(mut self, name: impl Into<String>, future: Fut) -> Self
    where
        Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>
            + Send
            + 'static,
    {
        self.task_manager
            .add_task(Box::new(SpawnTask::new(name, future)));
        self
    }

    /// 添加 spawn 任务（带依赖）
    pub fn add_spawn_with_deps<Fut>(
        mut self,
        name: impl Into<String>,
        future: Fut,
        dependencies: Vec<String>,
    ) -> Self
    where
        Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>
            + Send
            + 'static,
    {
        self.task_manager.add_task(Box::new(
            SpawnTask::new(name, future).with_dependencies(dependencies),
        ));
        self
    }

    /// 添加 spawn 任务（需要 shutdown）
    pub fn add_spawn_with_shutdown<F, Fut>(mut self, name: impl Into<String>, future_fn: F) -> Self
    where
        F: FnOnce(oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>
            + Send
            + 'static,
    {
        self.task_manager
            .add_task(Box::new(SpawnTask::with_shutdown(name, future_fn)));
        self
    }

    /// 获取状态追踪器
    pub fn state_tracker(&self) -> Arc<StateTracker> {
        self.task_manager.state_tracker()
    }

    /// 启动健康检查监控任务
    fn start_health_monitor(&mut self) -> Option<HealthMonitorHandle> {
        if !self.config.health_check.enabled {
            return None;
        }

        let mut checker = self.health_checker.take().unwrap_or_else(|| {
            let mut default_checker = HealthChecker::new()
                .with_failure_threshold(self.config.health_check.failure_threshold);
            default_checker.add_check(Arc::new(TaskFailureHealthCheck::new(
                self.task_manager.state_tracker(),
            )));
            default_checker
        });

        if checker.check_count() == 0 {
            checker.add_check(Arc::new(TaskFailureHealthCheck::new(
                self.task_manager.state_tracker(),
            )));
        }

        let mut failure_rx = None;
        if self.health_failure_action == HealthFailureAction::GracefulShutdown {
            let (failure_tx, rx) = mpsc::unbounded_channel::<String>();
            checker = checker.with_on_failure(Arc::new(move |check_name: &str| {
                let _ = failure_tx.send(check_name.to_string());
            }));
            failure_rx = Some(rx);
        }

        let service_name = self.service_name.clone();
        let interval = self.config.health_check.interval;
        let timeout = self.config.health_check.timeout;
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            info!(
                service_name = %service_name,
                interval_ms = interval.as_millis() as u64,
                timeout_ms = timeout.as_millis() as u64,
                check_count = checker.check_count(),
                "Health monitor started"
            );

            loop {
                tokio::select! {
                    _ = &mut stop_rx => {
                        info!(service_name = %service_name, "Health monitor stopped");
                        break;
                    }
                    _ = ticker.tick() => {
                        match tokio::time::timeout(timeout, checker.check_all()).await {
                            Ok(results) => {
                                let unhealthy: Vec<_> = results.into_iter().filter(|r| !r.healthy).collect();
                                if !unhealthy.is_empty() {
                                    let names: Vec<_> = unhealthy.into_iter().map(|r| r.name).collect();
                                    warn!(
                                        service_name = %service_name,
                                        unhealthy_checks = ?names,
                                        "Health monitor detected unhealthy checks"
                                    );
                                }
                            }
                            Err(_) => {
                                warn!(
                                    service_name = %service_name,
                                    timeout_ms = timeout.as_millis() as u64,
                                    "Health monitor round timed out"
                                );
                            }
                        }
                    }
                }
            }
        });

        Some(HealthMonitorHandle {
            stop_tx,
            join_handle: handle,
            failure_rx,
        })
    }

    /// 运行服务（简单模式，不注册服务）
    ///
    /// 执行以下步骤：
    /// 1. 启动所有任务
    /// 2. 等待所有任务就绪
    /// 3. 等待关闭信号（Ctrl+C）
    /// 4. 优雅关闭所有任务
    pub async fn run(self) -> Result<()> {
        self.run_with_signals(vec![]).await
    }

    /// 运行服务（带自定义信号）
    ///
    /// # 参数
    ///
    /// * `signals` - 自定义停机信号列表
    pub async fn run_with_signals(
        mut self,
        mut signals: Vec<Box<dyn ShutdownSignal>>,
    ) -> Result<()> {
        info!(
            service_name = %self.service_name,
            task_count = self.task_manager.task_count(),
            "🚀 Starting service runtime"
        );

        // 1. 构建停机信号
        if signals.is_empty() {
            // 默认信号：Ctrl+C + SIGTERM (Unix)
            signals.push(Box::new(CtrlCSignal::new()));

            #[cfg(target_family = "unix")]
            signals.push(Box::new(UnixSignal::new(UnixSignalKind::Terminate)));
        }

        let mut shutdown_signal = CompositeSignal::from_signals(signals);

        // 2. 启动所有任务
        let (join_set, shutdown_txs) = self
            .task_manager
            .start_all()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start tasks: {}", e))?;

        // 3. 等待所有任务就绪
        self.task_manager
            .wait_for_ready()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to wait for tasks ready: {}", e))?;

        // 4.1 启动健康检查监控（可选）
        let mut health_monitor = self.start_health_monitor();

        // 5. 等待停机信号或健康检查触发停机
        info!("Waiting for shutdown signal...");
        if let Some(monitor) = health_monitor.as_mut() {
            if let Some(failure_rx) = monitor.failure_rx.as_mut() {
                tokio::select! {
                    _ = shutdown_signal.wait() => {
                        info!("Shutdown signal received");
                    }
                    failed = failure_rx.recv() => {
                        warn!(failed_check = ?failed, "Health check threshold exceeded, triggering graceful shutdown");
                    }
                }
            } else {
                shutdown_signal.wait().await;
                info!("Shutdown signal received");
            }
        } else {
            shutdown_signal.wait().await;
            info!("Shutdown signal received");
        }

        // 6. 停止健康检查监控
        if let Some(monitor) = health_monitor {
            let _ = monitor.stop_tx.send(());
            let _ = monitor.join_handle.await;
        }

        // 7. 停止所有任务
        self.task_manager.stop_all(join_set, shutdown_txs).await;

        info!(service_name = %self.service_name, "Service runtime stopped");
        Ok(())
    }

    /// 运行服务（带服务注册）
    ///
    /// 执行以下步骤：
    /// 1. 启动所有任务
    /// 2. 等待所有任务就绪
    /// 3. 注册服务
    /// 4. 等待关闭信号
    /// 5. 注销服务
    /// 6. 优雅关闭所有任务
    pub async fn run_with_registration<F, Fut>(mut self, register_fn: F) -> Result<()>
    where
        F: FnOnce(SocketAddr) -> Fut,
        Fut: std::future::Future<
                Output = Result<
                    Option<Box<dyn ServiceRegistry>>,
                    Box<dyn std::error::Error + Send + Sync>,
                >,
            > + Send,
    {
        let service_name = self.service_name.clone();
        let service_address = self.service_address.ok_or_else(|| {
            anyhow::anyhow!(
                "Service address is required for service registration. \
                 Use `with_address()` to set the address."
            )
        })?;

        info!(
            service_name = %service_name,
            address = %service_address,
            task_count = self.task_manager.task_count(),
            "🚀 Starting service runtime with registration"
        );

        // 1. 构建停机信号
        let mut shutdown_signal = CompositeSignal::new().add(Box::new(CtrlCSignal::new()));

        #[cfg(target_family = "unix")]
        {
            shutdown_signal =
                shutdown_signal.add(Box::new(UnixSignal::new(UnixSignalKind::Terminate)));
        }

        // 2. 启动所有任务
        let (join_set, shutdown_txs) = self
            .task_manager
            .start_all()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start tasks: {}", e))?;

        // 3. 等待所有任务就绪
        self.task_manager
            .wait_for_ready()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to wait for tasks ready: {}", e))?;

        // 4. 注册服务
        info!("Registering service...");
        let registry = match register_fn(service_address).await {
            Ok(Some(reg)) => {
                info!("✅ Service registered: {}", service_name);
                Some(reg)
            }
            Ok(None) => {
                info!("Service registration skipped");
                None
            }
            Err(e) => {
                error!(error = %e, "❌ Service registration failed");

                // 停止所有任务
                self.task_manager.stop_all(join_set, shutdown_txs).await;

                return Err(anyhow::anyhow!("Service registration failed: {}", e));
            }
        };

        // 4.1 启动健康检查监控（可选）
        let mut health_monitor = self.start_health_monitor();

        // 5. 等待停机信号或健康检查触发停机
        info!("Waiting for shutdown signal...");
        if let Some(monitor) = health_monitor.as_mut() {
            if let Some(failure_rx) = monitor.failure_rx.as_mut() {
                tokio::select! {
                    _ = shutdown_signal.wait() => {
                        info!("Shutdown signal received");
                    }
                    failed = failure_rx.recv() => {
                        warn!(failed_check = ?failed, "Health check threshold exceeded, triggering graceful shutdown");
                    }
                }
            } else {
                shutdown_signal.wait().await;
                info!("Shutdown signal received");
            }
        } else {
            shutdown_signal.wait().await;
            info!("Shutdown signal received");
        }

        // 5.1 停止健康检查监控
        if let Some(monitor) = health_monitor {
            let _ = monitor.stop_tx.send(());
            let _ = monitor.join_handle.await;
        }

        // 6. 注销服务
        if let Some(mut reg) = registry {
            info!("Deregistering service...");
            if let Err(e) = reg.shutdown().await {
                warn!(error = %e, "⚠️ Failed to deregister service gracefully");
            } else {
                info!("✅ Service deregistered");
            }
        }

        // 7. 停止所有任务
        self.task_manager.stop_all(join_set, shutdown_txs).await;

        info!(service_name = %self.service_name, "Service runtime stopped");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_runtime_new() {
        let runtime = ServiceRuntime::new("test-service");
        assert_eq!(runtime.service_name, "test-service");
    }

    #[test]
    fn test_service_runtime_simple() {
        let runtime = ServiceRuntime::simple();
        assert_eq!(runtime.service_name, "simple-runtime");
        assert!(runtime.service_address.is_none());
    }

    #[test]
    fn test_service_runtime_mq_consumer() {
        let runtime = ServiceRuntime::mq_consumer().add_spawn("kafka-consumer", async { Ok(()) });

        assert_eq!(runtime.task_manager.task_count(), 1);
    }

    #[test]
    fn test_service_runtime_tasks() {
        let runtime = ServiceRuntime::tasks()
            .add_spawn("task-1", async { Ok(()) })
            .add_spawn("task-2", async { Ok(()) });

        assert_eq!(runtime.task_manager.task_count(), 2);
    }

    #[test]
    fn test_service_runtime_with_address() {
        let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let runtime = ServiceRuntime::new("test-service").with_address(addr);

        assert_eq!(runtime.service_address, Some(addr));
    }

    #[test]
    fn test_service_runtime_add_spawn() {
        let runtime = ServiceRuntime::new("test-service").add_spawn("task-1", async { Ok(()) });

        assert_eq!(runtime.task_manager.task_count(), 1);
    }
}
