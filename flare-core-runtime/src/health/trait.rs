//! HealthCheck trait 定义
//!
//! 健康检查抽象，支持自定义健康检查逻辑

use crate::error::HealthError;
use std::future::Future;
use std::pin::Pin;

/// 健康检查抽象
///
/// 支持自定义健康检查逻辑
///
/// # 实现说明
///
/// - 使用返回 `Pin<Box<dyn Future>>` 的方式支持 trait object
/// - 所有健康检查器必须实现此 trait
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::health::HealthCheck;
/// use flare_core_runtime::error::HealthError;
/// use std::pin::Pin;
/// use std::future::Future;
///
/// struct DatabaseHealthCheck;
///
/// impl HealthCheck for DatabaseHealthCheck {
///     fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), HealthError>> + Send + '_>> {
///         Box::pin(async move {
///             // 检查数据库连接
///             Ok(())
///         })
///     }
///
///     fn name(&self) -> &str {
///         "database"
///     }
/// }
/// ```
pub trait HealthCheck: Send + Sync {
    /// 执行健康检查
    ///
    /// # 返回
    ///
    /// - `Ok(())` - 健康检查通过
    /// - `Err(HealthError)` - 健康检查失败
    fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), HealthError>> + Send + '_>>;

    /// 获取检查名称
    fn name(&self) -> &str;
}
