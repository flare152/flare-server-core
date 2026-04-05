//! 状态事件定义

use crate::task::TaskState;
use std::time::Instant;

/// 状态事件
///
/// 当任务状态变更时发出的事件
#[derive(Debug, Clone)]
pub struct StateEvent {
    /// 任务名称
    pub task_name: String,
    /// 旧状态
    pub old_state: TaskState,
    /// 新状态
    pub new_state: TaskState,
    /// 事件时间
    pub timestamp: Instant,
    /// 错误信息（如果状态变更为 Failed）
    pub error: Option<String>,
}

impl StateEvent {
    /// 创建新的状态事件
    pub fn new(task_name: impl Into<String>, old_state: TaskState, new_state: TaskState) -> Self {
        Self {
            task_name: task_name.into(),
            old_state,
            new_state,
            timestamp: Instant::now(),
            error: None,
        }
    }

    /// 添加错误信息
    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    /// 检查是否为失败事件
    pub fn is_failure(&self) -> bool {
        self.new_state == TaskState::Failed
    }

    /// 检查是否为终态事件
    pub fn is_terminal(&self) -> bool {
        self.new_state.is_terminal()
    }
}

/// 运行时事件
///
/// 运行时级别的事件
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// 运行时启动
    Startup {
        /// 运行时名称
        name: String,
        /// 启动时间
        timestamp: Instant,
    },

    /// 运行时停机
    Shutdown {
        /// 运行时名称
        name: String,
        /// 停机时间
        timestamp: Instant,
    },

    /// 任务状态变更
    TaskStateChanged(StateEvent),

    /// 健康检查失败
    HealthCheckFailed {
        /// 检查名称
        check_name: String,
        /// 失败原因
        reason: String,
        /// 时间
        timestamp: Instant,
    },

    /// 服务注册成功
    ServiceRegistered {
        /// 服务名称
        service_name: String,
        /// 服务 ID
        service_id: String,
        /// 时间
        timestamp: Instant,
    },

    /// 服务注销
    ServiceDeregistered {
        /// 服务名称
        service_name: String,
        /// 服务 ID
        service_id: String,
        /// 时间
        timestamp: Instant,
    },
}

impl RuntimeEvent {
    /// 获取事件时间
    pub fn timestamp(&self) -> Instant {
        match self {
            RuntimeEvent::Startup { timestamp, .. } => *timestamp,
            RuntimeEvent::Shutdown { timestamp, .. } => *timestamp,
            RuntimeEvent::TaskStateChanged(event) => event.timestamp,
            RuntimeEvent::HealthCheckFailed { timestamp, .. } => *timestamp,
            RuntimeEvent::ServiceRegistered { timestamp, .. } => *timestamp,
            RuntimeEvent::ServiceDeregistered { timestamp, .. } => *timestamp,
        }
    }
}
