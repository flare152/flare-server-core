//! ShutdownSignal trait 定义
//!
//! 停机信号抽象，支持多种信号源：Ctrl+C, SIGTERM, 自定义通道等

use std::future::Future;
use std::pin::Pin;

/// 停机信号抽象
///
/// 支持多种信号源：Ctrl+C, SIGTERM, 自定义通道等
///
/// # 实现说明
///
/// - 使用 Boxed Future 模式支持 trait object
/// - 所有信号源必须实现此 trait
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::signal::ShutdownSignal;
/// use std::pin::Pin;
/// use std::future::Future;
///
/// struct CtrlCSignal;
///
/// impl ShutdownSignal for CtrlCSignal {
///     fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
///         Box::pin(async {
///             let _ = tokio::signal::ctrl_c().await;
///         })
///     }
///
///     fn name(&self) -> &str {
///         "ctrl_c"
///     }
/// }
/// ```
pub trait ShutdownSignal: Send {
    /// 等待信号触发
    ///
    /// 当信号触发时，此方法返回
    fn wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// 获取信号名称 (用于日志)
    fn name(&self) -> &str;
}
