//! Middleware trait 定义
//!
//! 中间件抽象，支持在任务执行前后插入自定义逻辑

use crate::error::MiddlewareError;
use async_trait::async_trait;

/// 中间件抽象
///
/// 支持在任务执行前后插入自定义逻辑
///
/// # 实现说明
///
/// - 使用 async-trait 宏支持 trait objects
/// - 所有中间件必须实现此 trait
/// - 中间件按照注册顺序执行
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::middleware::Middleware;
/// use flare_core_runtime::error::MiddlewareError;
///
/// struct LoggingMiddleware;
///
/// impl Middleware for LoggingMiddleware {
///     fn name(&self) -> &str {
///         "logging"
///     }
///
///     async fn before(&self, task_name: &str) -> Result<(), MiddlewareError> {
///         tracing::info!("Task {} starting", task_name);
///         Ok(())
///     }
///
///     async fn after(&self, task_name: &str, result: &Result<(), Box<dyn std::error::Error + Send + Sync>>) -> Result<(), MiddlewareError> {
///         tracing::info!("Task {} completed", task_name);
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Middleware: Send + Sync {
    /// 获取中间件名称
    fn name(&self) -> &str;

    /// 任务执行前调用
    ///
    /// # 参数
    ///
    /// * `task_name` - 任务名称
    ///
    /// # 返回
    ///
    /// - `Ok(())` - 继续执行
    /// - `Err(MiddlewareError)` - 中断中间件链
    async fn before(&self, task_name: &str) -> Result<(), MiddlewareError> {
        let _ = task_name;
        Ok(())
    }

    /// 任务执行后调用
    ///
    /// # 参数
    ///
    /// * `task_name` - 任务名称
    /// * `result` - 任务执行结果
    ///
    /// # 返回
    ///
    /// - `Ok(())` - 继续执行
    /// - `Err(MiddlewareError)` - 中断中间件链
    async fn after(
        &self,
        task_name: &str,
        result: &Result<(), Box<dyn std::error::Error + Send + Sync>>,
    ) -> Result<(), MiddlewareError> {
        let _ = (task_name, result);
        Ok(())
    }
}
