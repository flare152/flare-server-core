//! MetricsCollector trait 定义
//!
//! 指标收集抽象，支持多种监控系统：Prometheus, Datadog 等

use crate::error::MetricsError;
use crate::task::TaskState;
use async_trait::async_trait;
use std::time::Duration;

/// 指标收集器抽象
///
/// 支持多种监控系统：Prometheus, Datadog 等
///
/// # 实现说明
///
/// - 使用 async-trait 宏支持 trait objects
/// - 所有指标收集器必须实现此 trait
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::metrics::MetricsCollector;
/// use flare_core_runtime::error::MetricsError;
/// use std::time::Duration;
///
/// struct PrometheusCollector;
///
/// impl MetricsCollector for PrometheusCollector {
///     async fn record_task_startup(&self, name: &str, duration: Duration) -> Result<(), MetricsError> {
///         // 记录任务启动耗时
///         Ok(())
///     }
///
///     async fn record_task_state(&self, name: &str, state: TaskState) -> Result<(), MetricsError> {
///         // 记录任务状态
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait MetricsCollector: Send + Sync {
    /// 记录任务启动耗时
    ///
    /// # 参数
    ///
    /// * `name` - 任务名称
    /// * `duration` - 启动耗时
    async fn record_task_startup(&self, name: &str, duration: Duration)
    -> Result<(), MetricsError>;

    /// 记录任务停机耗时
    ///
    /// # 参数
    ///
    /// * `name` - 任务名称
    /// * `duration` - 停机耗时
    async fn record_task_shutdown(
        &self,
        name: &str,
        duration: Duration,
    ) -> Result<(), MetricsError>;

    /// 记录任务状态变更
    ///
    /// # 参数
    ///
    /// * `name` - 任务名称
    /// * `state` - 任务状态
    async fn record_task_state(&self, name: &str, state: TaskState) -> Result<(), MetricsError>;

    /// 记录健康检查结果
    ///
    /// # 参数
    ///
    /// * `name` - 健康检查名称
    /// * `success` - 是否成功
    async fn record_health_check(&self, name: &str, success: bool) -> Result<(), MetricsError>;

    /// 导出指标
    ///
    /// # 返回
    ///
    /// 返回指标数据（格式由实现决定，如 Prometheus 文本格式）
    async fn export(&self) -> Result<String, MetricsError>;
}
