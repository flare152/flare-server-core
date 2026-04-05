//! 状态追踪器实现

use super::event::StateEvent;
use crate::task::TaskState;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, broadcast};

/// 任务状态信息
#[derive(Debug, Clone)]
pub struct TaskStateInfo {
    /// 任务名称
    pub name: String,
    /// 当前状态
    pub state: TaskState,
    /// 状态变更时间
    pub state_changed_at: Instant,
    /// 启动时间（如果已启动）
    pub started_at: Option<Instant>,
    /// 运行时长（如果正在运行）
    pub running_duration: Option<Duration>,
}

impl TaskStateInfo {
    /// 创建新的任务状态信息
    pub fn new(name: impl Into<String>, initial_state: TaskState) -> Self {
        Self {
            name: name.into(),
            state: initial_state,
            state_changed_at: Instant::now(),
            started_at: None,
            running_duration: None,
        }
    }

    /// 更新状态
    pub fn update_state(&mut self, new_state: TaskState) {
        let now = Instant::now();

        // 记录启动时间
        if new_state == TaskState::Running && self.started_at.is_none() {
            self.started_at = Some(now);
        }

        // 计算运行时长
        if new_state == TaskState::Running {
            if let Some(started_at) = self.started_at {
                self.running_duration = Some(now.duration_since(started_at));
            }
        } else {
            self.running_duration = None;
        }

        self.state = new_state;
        self.state_changed_at = now;
    }
}

/// 状态追踪器
///
/// 实时追踪所有任务的状态，支持事件订阅
///
/// # 特性
///
/// - 线程安全（使用 `Arc<RwLock>`）
/// - 支持事件订阅（使用 `broadcast` 通道）
/// - 记录状态变更时间
/// - 计算运行时长
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::state::StateTracker;
/// use flare_core_runtime::task::TaskState;
///
/// let tracker = StateTracker::new();
///
/// // 注册任务
/// tracker.register_task("task-1", TaskState::Pending);
///
/// // 更新状态
/// tracker.update_state("task-1", TaskState::Running);
///
/// // 获取状态
/// let info = tracker.get_state("task-1").unwrap();
/// assert_eq!(info.state, TaskState::Running);
/// ```
pub struct StateTracker {
    /// 任务状态映射
    tasks: Arc<RwLock<HashMap<String, TaskStateInfo>>>,
    /// 事件发送器
    event_tx: broadcast::Sender<StateEvent>,
}

impl StateTracker {
    /// 创建新的状态追踪器
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// 创建新的状态追踪器（指定事件缓冲区大小）
    pub fn with_capacity(capacity: usize) -> Self {
        let (event_tx, _) = broadcast::channel(capacity);
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// 注册任务
    pub async fn register_task(&self, name: impl Into<String>, initial_state: TaskState) {
        let name = name.into();
        let info = TaskStateInfo::new(&name, initial_state);

        let mut tasks = self.tasks.write().await;
        tasks.insert(name, info);
    }

    /// 更新任务状态
    ///
    /// # 返回
    ///
    /// 返回状态事件（如果状态变更成功）
    pub async fn update_state(&self, name: &str, new_state: TaskState) -> Option<StateEvent> {
        let mut tasks = self.tasks.write().await;

        if let Some(info) = tasks.get_mut(name) {
            let old_state = info.state;

            // 检查状态转换是否有效
            if !old_state.can_transition_to(new_state) {
                return None;
            }

            // 更新状态
            info.update_state(new_state);

            // 创建事件
            let event = StateEvent::new(name, old_state, new_state);

            // 发送事件（忽略错误，如果没有订阅者）
            let _ = self.event_tx.send(event.clone());

            return Some(event);
        }

        None
    }

    /// 更新任务状态（带错误信息）
    pub async fn update_state_with_error(
        &self,
        name: &str,
        new_state: TaskState,
        error: impl Into<String>,
    ) -> Option<StateEvent> {
        let mut tasks = self.tasks.write().await;

        if let Some(info) = tasks.get_mut(name) {
            let old_state = info.state;

            // 检查状态转换是否有效
            if !old_state.can_transition_to(new_state) {
                return None;
            }

            // 更新状态
            info.update_state(new_state);

            // 创建事件
            let event = StateEvent::new(name, old_state, new_state).with_error(error);

            // 发送事件
            let _ = self.event_tx.send(event.clone());

            return Some(event);
        }

        None
    }

    /// 获取任务状态
    pub async fn get_state(&self, name: &str) -> Option<TaskStateInfo> {
        let tasks = self.tasks.read().await;
        tasks.get(name).cloned()
    }

    /// 获取所有任务状态
    pub async fn get_all_states(&self) -> Vec<TaskStateInfo> {
        let tasks = self.tasks.read().await;
        tasks.values().cloned().collect()
    }

    /// 订阅状态事件
    pub fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.event_tx.subscribe()
    }

    /// 检查所有任务是否都已就绪
    pub async fn all_ready(&self) -> bool {
        let tasks = self.tasks.read().await;
        tasks.values().all(|info| info.state == TaskState::Running)
    }

    /// 检查是否有任务失败
    pub async fn has_failures(&self) -> bool {
        let tasks = self.tasks.read().await;
        tasks.values().any(|info| info.state == TaskState::Failed)
    }

    /// 获取失败的任务
    pub async fn get_failed_tasks(&self) -> Vec<String> {
        let tasks = self.tasks.read().await;
        tasks
            .values()
            .filter(|info| info.state == TaskState::Failed)
            .map(|info| info.name.clone())
            .collect()
    }

    /// 获取运行中的任务数量
    pub async fn running_count(&self) -> usize {
        let tasks = self.tasks.read().await;
        tasks
            .values()
            .filter(|info| info.state == TaskState::Running)
            .count()
    }
}

impl Default for StateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for StateTracker {
    fn clone(&self) -> Self {
        Self {
            tasks: Arc::clone(&self.tasks),
            event_tx: self.event_tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_state_tracker_register() {
        let tracker = StateTracker::new();
        tracker.register_task("task-1", TaskState::Pending).await;

        let info = tracker.get_state("task-1").await.unwrap();
        assert_eq!(info.state, TaskState::Pending);
    }

    #[tokio::test]
    async fn test_state_tracker_update() {
        let tracker = StateTracker::new();
        tracker.register_task("task-1", TaskState::Pending).await;

        let event = tracker.update_state("task-1", TaskState::Starting).await;
        assert!(event.is_some());

        let info = tracker.get_state("task-1").await.unwrap();
        assert_eq!(info.state, TaskState::Starting);
    }

    #[tokio::test]
    async fn test_state_tracker_invalid_transition() {
        let tracker = StateTracker::new();
        tracker.register_task("task-1", TaskState::Pending).await;

        // 无效转换：Pending -> Running（应该先 Starting）
        let event = tracker.update_state("task-1", TaskState::Running).await;
        assert!(event.is_none());
    }

    #[tokio::test]
    async fn test_state_tracker_subscribe() {
        let tracker = StateTracker::new();
        let mut rx = tracker.subscribe();

        tracker.register_task("task-1", TaskState::Pending).await;

        // 在另一个任务中更新状态
        let tracker_clone = tracker.clone();
        tokio::spawn(async move {
            tracker_clone
                .update_state("task-1", TaskState::Starting)
                .await;
        });

        // 接收事件
        let event = rx.recv().await.unwrap();
        assert_eq!(event.task_name, "task-1");
        assert_eq!(event.old_state, TaskState::Pending);
        assert_eq!(event.new_state, TaskState::Starting);
    }

    #[tokio::test]
    async fn test_state_tracker_all_ready() {
        let tracker = StateTracker::new();

        tracker.register_task("task-1", TaskState::Pending).await;
        tracker.register_task("task-2", TaskState::Pending).await;

        assert!(!tracker.all_ready().await);

        tracker.update_state("task-1", TaskState::Running).await;
        tracker.update_state("task-2", TaskState::Running).await;

        assert!(tracker.all_ready().await);
    }
}
