//! 任务管理器实现
//!
//! 负责管理所有任务的生命周期，包括启动、停止、依赖管理等

use super::{Task, TaskResult, TaskState};
use crate::config::RuntimeConfig;
use crate::error::RuntimeError;
use crate::state::StateTracker;
use crate::utils::topological_sort;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

/// 任务管理器
///
/// 负责管理所有任务的生命周期
///
/// # 功能
///
/// - 任务注册和管理
/// - 依赖排序和启动
/// - 优雅停机
/// - 状态追踪
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::task::TaskManager;
/// use flare_core_runtime::task::SpawnTask;
///
/// let manager = TaskManager::new();
///
/// // 添加任务
/// manager.add_task(Box::new(SpawnTask::new("task-1", async { Ok(()) })));
///
/// // 启动所有任务
/// let (join_set, shutdown_txs) = manager.start_all().await?;
///
/// // 等待停机信号
/// // ...
///
/// // 停止所有任务
/// manager.stop_all(join_set, shutdown_txs).await;
/// ```
pub struct TaskManager {
    /// 任务列表
    tasks: Vec<Box<dyn Task>>,
    /// 状态追踪器
    state_tracker: Arc<StateTracker>,
    /// 配置
    config: RuntimeConfig,
}

impl TaskManager {
    /// 创建新的任务管理器
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            state_tracker: Arc::new(StateTracker::new()),
            config: RuntimeConfig::default(),
        }
    }

    /// 创建新的任务管理器（带配置）
    pub fn with_config(config: RuntimeConfig) -> Self {
        Self {
            tasks: Vec::new(),
            state_tracker: Arc::new(StateTracker::new()),
            config,
        }
    }

    /// 添加任务
    pub fn add_task(&mut self, task: Box<dyn Task>) {
        info!(task_name = %task.name(), "Adding task to manager");
        self.tasks.push(task);
    }

    /// 获取任务数量
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// 获取状态追踪器
    pub fn state_tracker(&self) -> Arc<StateTracker> {
        Arc::clone(&self.state_tracker)
    }

    /// 启动所有任务
    ///
    /// # 返回
    ///
    /// - `join_set` - 任务 JoinSet，用于等待任务完成
    /// - `shutdown_txs` - shutdown 信号发送器列表
    ///
    /// # 错误
    ///
    /// - 循环依赖
    /// - 缺失依赖
    pub async fn start_all(
        &mut self,
    ) -> Result<(JoinSet<TaskResult>, Vec<oneshot::Sender<()>>), RuntimeError> {
        info!(task_count = self.tasks.len(), "Starting all tasks");

        // 1. 拓扑排序，确定启动顺序
        let sorted_tasks = self.sort_tasks()?;

        // 2. 注册所有任务到状态追踪器
        for task in &sorted_tasks {
            self.state_tracker
                .register_task(task.name(), TaskState::Pending)
                .await;
        }

        // 3. 启动任务
        let mut join_set = JoinSet::new();
        let mut shutdown_txs = Vec::new();

        for task in sorted_tasks {
            let task_name = task.name().to_string();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            shutdown_txs.push(shutdown_tx);

            // 更新状态为 Starting
            self.state_tracker
                .update_state(&task_name, TaskState::Starting)
                .await;

            // 启动任务
            let state_tracker = Arc::clone(&self.state_tracker);
            let task_future = task.run(shutdown_rx);

            join_set.spawn(async move {
                // 更新状态为 Running
                state_tracker
                    .update_state(&task_name, TaskState::Running)
                    .await;

                // 执行任务
                let result = task_future.await;

                // 更新状态
                match &result {
                    Ok(_) => {
                        info!(task_name = %task_name, "✅ Task completed");
                        state_tracker
                            .update_state(&task_name, TaskState::Stopped)
                            .await;
                    }
                    Err(e) => {
                        error!(task_name = %task_name, error = %e, "❌ Task failed");
                        state_tracker
                            .update_state_with_error(&task_name, TaskState::Failed, e.to_string())
                            .await;
                    }
                }

                result
            });
        }

        info!("All tasks started");
        Ok((join_set, shutdown_txs))
    }

    /// 停止所有任务
    ///
    /// # 参数
    ///
    /// * `join_set` - 任务 JoinSet
    /// * `shutdown_txs` - shutdown 信号发送器列表
    pub async fn stop_all(
        &self,
        mut join_set: JoinSet<TaskResult>,
        shutdown_txs: Vec<oneshot::Sender<()>>,
    ) {
        info!("Stopping all tasks");

        // 1. 发送 shutdown 信号
        for tx in shutdown_txs {
            let _ = tx.send(());
        }

        // 2. 等待所有任务关闭
        match tokio::time::timeout(self.config.shutdown_timeout, async {
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(Ok(_)) => {
                        info!("Task completed gracefully");
                    }
                    Ok(Err(e)) => {
                        warn!("Task completed with error: {}", e);
                    }
                    Err(e) => {
                        warn!("Task join error: {}", e);
                    }
                }
            }
        })
        .await
        {
            Ok(_) => {
                info!("All tasks completed");
            }
            Err(_) => {
                warn!("Tasks shutdown timeout, forcing exit");
                join_set.abort_all();
            }
        }
    }

    /// 等待所有任务就绪
    pub async fn wait_for_ready(&self) -> Result<(), RuntimeError> {
        if !self.config.task_startup.enable_ready_check {
            info!("Task ready check is disabled, skipping");
            return Ok(());
        }

        info!("Waiting for all tasks to be ready");

        // 等待所有任务状态变为 Running
        let timeout = self.config.task_startup.ready_check_timeout;
        let start = std::time::Instant::now();

        loop {
            if self.state_tracker.all_ready().await {
                info!("✅ All tasks are ready");
                return Ok(());
            }

            if start.elapsed() > timeout {
                return Err(RuntimeError::StartupTimeout {
                    name: "all tasks".to_string(),
                    timeout,
                });
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// 拓扑排序任务
    fn sort_tasks(&mut self) -> Result<Vec<Box<dyn Task>>, RuntimeError> {
        // 构建任务依赖列表
        let items: Vec<(String, Vec<String>)> = self
            .tasks
            .iter()
            .map(|task| (task.name().to_string(), task.dependencies()))
            .collect();

        // 拓扑排序
        let sorted_names = topological_sort(items)
            .map_err(|cycle| RuntimeError::CircularDependency { tasks: cycle })?;

        // 按拓扑名顺序取出任务：禁止用「原始下标 + swap_remove」——每次 swap_remove 都会打乱下标，导致越界 panic。
        let mut by_name: HashMap<String, Box<dyn Task>> = HashMap::new();
        for task in self.tasks.drain(..) {
            by_name.insert(task.name().to_string(), task);
        }

        let mut sorted_tasks = Vec::with_capacity(sorted_names.len());
        for name in sorted_names {
            if let Some(task) = by_name.remove(&name) {
                sorted_tasks.push(task);
            }
        }
        for (_, task) in by_name {
            warn!(
                task_name = %task.name(),
                "Task missing from topological order output, appending at end"
            );
            sorted_tasks.push(task);
        }

        Ok(sorted_tasks)
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::SpawnTask;

    #[tokio::test]
    async fn test_task_manager_new() {
        let manager = TaskManager::new();
        assert_eq!(manager.task_count(), 0);
    }

    #[tokio::test]
    async fn test_task_manager_add_task() {
        let mut manager = TaskManager::new();
        manager.add_task(Box::new(SpawnTask::new("task-1", async { Ok(()) })));
        assert_eq!(manager.task_count(), 1);
    }

    #[tokio::test]
    async fn test_task_manager_state_tracker() {
        let manager = TaskManager::new();
        let _tracker = manager.state_tracker();
    }

    /// 回归：多任务且无依赖时，拓扑序与插入序可能不一致，旧实现用 swap_remove(原下标) 会 panic。
    #[tokio::test]
    async fn test_task_manager_start_all_three_independent_no_panic() {
        let mut manager = TaskManager::new();
        manager.add_task(Box::new(SpawnTask::new("conversation-grpc", async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(())
        })));
        manager.add_task(Box::new(SpawnTask::new("read-receipt-consumer", async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(())
        })));
        manager.add_task(Box::new(SpawnTask::new("conversation-ensure-consumer", async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(())
        })));

        let (mut join_set, shutdown_txs) = manager.start_all().await.expect("start_all");
        for tx in shutdown_txs {
            let _ = tx.send(());
        }
        while join_set.join_next().await.is_some() {}
    }
}
