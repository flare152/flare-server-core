//! 运行时配置模块

use std::time::Duration;

/// 运行时配置
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// 关闭超时时间（默认 5 秒）
    pub shutdown_timeout: Duration,
    /// 就绪检查超时时间（默认 30 秒）
    pub ready_check_timeout: Duration,
    /// 是否启用任务就绪检查（默认 true）
    pub enable_task_ready_check: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            shutdown_timeout: Duration::from_secs(5),
            ready_check_timeout: Duration::from_secs(30),
            enable_task_ready_check: true,
        }
    }
}

impl RuntimeConfig {
    /// 创建默认配置
    pub fn new() -> Self {
        Self::default()
    }
    
    /// 设置关闭超时时间
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }
    
    /// 设置就绪检查超时时间
    pub fn with_ready_check_timeout(mut self, timeout: Duration) -> Self {
        self.ready_check_timeout = timeout;
        self
    }
    
    /// 启用/禁用任务就绪检查
    pub fn with_task_ready_check(mut self, enable: bool) -> Self {
        self.enable_task_ready_check = enable;
        self
    }
}

