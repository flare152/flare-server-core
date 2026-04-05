//! 任务状态定义

use std::fmt;

/// 任务状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskState {
    /// 等待启动
    Pending,
    /// 启动中
    Starting,
    /// 运行中
    Running,
    /// 停止中
    Stopping,
    /// 已停止
    Stopped,
    /// 失败
    Failed,
}

impl TaskState {
    /// 检查是否为终态（Stopped 或 Failed）
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskState::Stopped | TaskState::Failed)
    }

    /// 检查是否可以转换到目标状态
    pub fn can_transition_to(&self, target: TaskState) -> bool {
        use TaskState::*;
        match (self, target) {
            // Pending 可以转换到 Starting
            (Pending, Starting) => true,
            // Starting 可以转换到 Running 或 Failed
            (Starting, Running) | (Starting, Failed) => true,
            // Running 可以转换到 Stopping 或 Failed
            (Running, Stopping) | (Running, Failed) => true,
            // Stopping 可以转换到 Stopped 或 Failed
            (Stopping, Stopped) | (Stopping, Failed) => true,
            // 终态不能转换
            (Stopped, _) | (Failed, _) => false,
            // 其他转换无效
            _ => false,
        }
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskState::Pending => write!(f, "Pending"),
            TaskState::Starting => write!(f, "Starting"),
            TaskState::Running => write!(f, "Running"),
            TaskState::Stopping => write!(f, "Stopping"),
            TaskState::Stopped => write!(f, "Stopped"),
            TaskState::Failed => write!(f, "Failed"),
        }
    }
}
