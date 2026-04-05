//! Ctrl+C 信号实现

use super::ShutdownSignal;
use std::pin::Pin;

/// Ctrl+C 信号
///
/// 监听用户按下 Ctrl+C 的信号
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::signal::CtrlCSignal;
///
/// let signal = CtrlCSignal::new();
/// ```
pub struct CtrlCSignal {
    name: String,
}

impl CtrlCSignal {
    /// 创建新的 Ctrl+C 信号
    pub fn new() -> Self {
        Self {
            name: "ctrl_c".to_string(),
        }
    }
}

impl Default for CtrlCSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownSignal for CtrlCSignal {
    fn wait(&mut self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async {
            let _ = tokio::signal::ctrl_c().await;
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ctrl_c_signal_new() {
        let signal = CtrlCSignal::new();
        assert_eq!(signal.name(), "ctrl_c");
    }
}
