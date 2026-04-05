//! 插件管理器实现
//!
//! 管理所有插件并在关键生命周期调用插件钩子

use super::{Plugin, PluginContext};
use crate::error::PluginError;
use std::sync::Arc;
use tracing::{error, warn};

/// 插件管理器
///
/// 管理所有插件并在关键生命周期调用插件钩子
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::plugin::{PluginManager, Plugin};
///
/// let mut manager = PluginManager::new();
/// manager.register(Box::new(MyPlugin));
///
/// // 调用生命周期钩子
/// manager.on_startup(&ctx).await?;
/// ```
pub struct PluginManager {
    plugins: Vec<Arc<dyn Plugin>>,
}

impl PluginManager {
    /// 创建新的插件管理器
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// 注册插件
    ///
    /// # 参数
    ///
    /// * `plugin` - 要注册的插件
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) {
        let name = plugin.name().to_string();
        self.plugins.push(plugin);
        tracing::info!(plugin_name = %name, "Plugin registered");
    }

    /// 运行时启动时调用
    ///
    /// 调用所有插件的 `on_startup` 钩子
    pub async fn on_startup(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_startup(ctx).await {
                error!(
                    plugin_name = %plugin.name(),
                    error = %e,
                    "Plugin on_startup failed"
                );
                // 插件失败不影响运行时，继续执行其他插件
            }
        }
        Ok(())
    }

    /// 任务启动前调用
    ///
    /// 调用所有插件的 `on_task_start` 钩子
    pub async fn on_task_start(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_task_start(ctx).await {
                warn!(
                    plugin_name = %plugin.name(),
                    error = %e,
                    "Plugin on_task_start failed"
                );
            }
        }
        Ok(())
    }

    /// 任务停止后调用
    ///
    /// 调用所有插件的 `on_task_stop` 钩子
    pub async fn on_task_stop(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_task_stop(ctx).await {
                warn!(
                    plugin_name = %plugin.name(),
                    error = %e,
                    "Plugin on_task_stop failed"
                );
            }
        }
        Ok(())
    }

    /// 运行时停机时调用
    ///
    /// 调用所有插件的 `on_shutdown` 钩子
    pub async fn on_shutdown(&self, ctx: &PluginContext) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_shutdown(ctx).await {
                warn!(
                    plugin_name = %plugin.name(),
                    error = %e,
                    "Plugin on_shutdown failed"
                );
            }
        }
        Ok(())
    }

    /// 错误发生时调用
    ///
    /// 调用所有插件的 `on_error` 钩子
    pub async fn on_error(&self, ctx: &PluginContext, error: &str) -> Result<(), PluginError> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_error(ctx, error).await {
                warn!(
                    plugin_name = %plugin.name(),
                    error = %e,
                    "Plugin on_error failed"
                );
            }
        }
        Ok(())
    }

    /// 获取插件数量
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskState;

    struct TestPlugin {
        name: String,
    }

    impl Plugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_plugin_manager_register() {
        let mut manager = PluginManager::new();
        manager.register(Arc::new(TestPlugin {
            name: "test-plugin".to_string(),
        }));

        assert_eq!(manager.plugin_count(), 1);
    }

    #[tokio::test]
    async fn test_plugin_manager_on_startup() {
        let mut manager = PluginManager::new();
        manager.register(Arc::new(TestPlugin {
            name: "test-plugin".to_string(),
        }));

        let ctx = PluginContext::new("test-runtime");
        let result = manager.on_startup(&ctx).await;
        assert!(result.is_ok());
    }
}
