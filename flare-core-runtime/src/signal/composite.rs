//! 多信号合并器实现

use super::ShutdownSignal;
use futures::future::select_all;
use std::pin::Pin;

/// 多信号合并器
///
/// 监听多个信号源，任一触发即返回
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::signal::{CompositeSignal, CtrlCSignal, ShutdownSignal};
///
/// let signal = CompositeSignal::new()
///     .add(Box::new(CtrlCSignal::new()));
///
/// // 或者
/// let signal = CompositeSignal::from_signals(vec![
///     Box::new(CtrlCSignal::new()),
/// ]);
/// ```
pub struct CompositeSignal {
    signals: Vec<Box<dyn ShutdownSignal>>,
    name: String,
}

impl CompositeSignal {
    /// 创建新的多信号合并器
    pub fn new() -> Self {
        Self {
            signals: Vec::new(),
            name: "composite".to_string(),
        }
    }

    /// 从信号列表创建
    pub fn from_signals(signals: Vec<Box<dyn ShutdownSignal>>) -> Self {
        Self {
            signals,
            name: "composite".to_string(),
        }
    }

    /// 添加信号
    pub fn add(mut self, signal: Box<dyn ShutdownSignal>) -> Self {
        self.signals.push(signal);
        self
    }

    /// 设置名称
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Default for CompositeSignal {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownSignal for CompositeSignal {
    fn wait(&mut self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        if self.signals.is_empty() {
            // 没有信号源，直接等待
            return Box::pin(std::future::pending::<()>());
        }

        // 使用 select_all 等待任一信号
        let futures: Vec<_> = self.signals.iter_mut().map(|s| s.wait()).collect();

        if futures.is_empty() {
            return Box::pin(std::future::pending::<()>());
        }

        // 使用 select_all 等待第一个触发的信号
        Box::pin(async move {
            let (result, _index, _others) = select_all(futures).await;
            result
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::CtrlCSignal;

    #[test]
    fn test_composite_signal_new() {
        let signal = CompositeSignal::new();
        assert_eq!(signal.name(), "composite");
    }

    #[test]
    fn test_composite_signal_add() {
        let signal = CompositeSignal::new()
            .add(Box::new(CtrlCSignal::new()))
            .add(Box::new(CtrlCSignal::new()));

        assert_eq!(signal.name(), "composite");
    }

    #[test]
    fn test_composite_signal_from_signals() {
        let signal = CompositeSignal::from_signals(vec![
            Box::new(CtrlCSignal::new()),
            Box::new(CtrlCSignal::new()),
        ]);

        assert_eq!(signal.name(), "composite");
    }
}
