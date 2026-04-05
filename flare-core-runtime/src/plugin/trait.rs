//! Plugin trait 定义
//!
//! 插件抽象，支持在关键生命周期事件注入自定义逻辑

use crate::error::PluginError;
use crate::task::TaskState;
use async_trait::async_trait;

/// 插件上下文
///
/// 提供插件执行时的上下文信息
#[derive(Debug, Clone)]
pub struct PluginContext {
    /// 运行时名称
    pub runtime_name: String,
    /// 任务名称（如果适用）
    pub task_name: Option<String>,
    /// 任务状态（如果适用）
    pub task_state: Option<TaskState>,
}

impl PluginContext {
    /// 创建新的插件上下文
    pub fn new(runtime_name: impl Into<String>) -> Self {
        Self {
            runtime_name: runtime_name.into(),
            task_name: None,
            task_state: None,
        }
    }

    /// 设置任务名称
    pub fn with_task(mut self, name: impl Into<String>, state: TaskState) -> Self {
        self.task_name = Some(name.into());
        self.task_state = Some(state);
        self
    }
}

/// 插件抽象
///
/// 支持在关键生命周期事件注入自定义逻辑
///
/// # 实现说明
///
/// - 使用 async-trait 宏支持 trait objects
/// - 所有插件必须实现此 trait
/// - 插件可以在运行时启动、任务启动/停止、停机等事件时执行
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::plugin::{Plugin, PluginContext};
/// use flare_core_runtime::error::PluginError;
///
/// struct LoggingPlugin;
///
/// impl Plugin for LoggingPlugin {
///     fn name(&self) -> &str {
///         "logging"
///     }
///
///     async fn on_startup(&self, ctx: &PluginContext) -> Result<(), PluginError> {
///         tracing::info!("Runtime starting: {}", ctx.runtime_name);
///         Ok(())
///     }
///
///     async fn on_shutdown(&self, ctx: &PluginContext) -> Result<(), PluginError> {
///         tracing::info!("Runtime shutting down: {}", ctx.runtime_name);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Plugin: Send + Sync {
    /// 获取插件名称
    fn name(&self) -> &str;

    /// 运行时启动时调用
    ///
    /// # 参数
    ///
    /// * `ctx` - 插件上下文
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `PluginError`
    async fn on_startup(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        let _ = ctx;
        Ok(())
    }

    /// 任务启动前调用
    ///
    /// # 参数
    ///
    /// * `ctx` - 插件上下文（包含任务名称和状态）
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `PluginError`
    async fn on_task_start(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        let _ = ctx;
        Ok(())
    }

    /// 任务停止后调用
    ///
    /// # 参数
    ///
    /// * `ctx` - 插件上下文（包含任务名称和状态）
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `PluginError`
    async fn on_task_stop(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        let _ = ctx;
        Ok(())
    }

    /// 运行时停机时调用
    ///
    /// # 参数
    ///
    /// * `ctx` - 插件上下文
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `PluginError`
    async fn on_shutdown(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        let _ = ctx;
        Ok(())
    }

    /// 错误发生时调用
    ///
    /// # 参数
    ///
    /// * `ctx` - 插件上下文
    /// * `error` - 错误信息
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `PluginError`
    async fn on_error(&self, ctx: &PluginContext, error: &str) -> Result<(), PluginError> {
        let _ = (ctx, error);
        Ok(())
    }
}
