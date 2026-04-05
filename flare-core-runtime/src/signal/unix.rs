//! Unix 信号实现 (SIGTERM/SIGINT)
//!
//! 仅在 Unix 平台上可用

use super::ShutdownSignal;
use std::pin::Pin;

/// Unix 信号类型
#[derive(Debug, Clone, Copy)]
pub enum UnixSignalKind {
    /// SIGTERM
    Terminate,
    /// SIGINT
    Interrupt,
}

/// Unix 信号
///
/// 监听 SIGTERM 或 SIGINT 信号
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::signal::unix::{UnixSignal, UnixSignalKind};
///
/// let signal = UnixSignal::new(UnixSignalKind::Terminate);
/// ```
#[cfg(target_family = "unix")]
pub struct UnixSignal {
    kind: UnixSignalKind,
    name: String,
}

#[cfg(target_family = "unix")]
impl UnixSignal {
    /// 创建新的 Unix 信号
    pub fn new(kind: UnixSignalKind) -> Self {
        let name = match kind {
            UnixSignalKind::Terminate => "sigterm",
            UnixSignalKind::Interrupt => "sigint",
        };
        Self {
            kind,
            name: name.to_string(),
        }
    }
}

#[cfg(target_family = "unix")]
impl ShutdownSignal for UnixSignal {
    fn wait(&mut self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        use tokio::signal::unix::{SignalKind, signal};

        let kind = match self.kind {
            UnixSignalKind::Terminate => SignalKind::terminate(),
            UnixSignalKind::Interrupt => SignalKind::interrupt(),
        };

        Box::pin(async move {
            let mut signal = signal(kind).expect("Failed to register signal handler");
            let _ = signal.recv().await;
        })
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(not(target_family = "unix"))]
pub struct UnixSignal {
    _kind: UnixSignalKind,
    name: String,
}

#[cfg(not(target_family = "unix"))]
impl UnixSignal {
    pub fn new(kind: UnixSignalKind) -> Self {
        let name = match kind {
            UnixSignalKind::Terminate => "sigterm",
            UnixSignalKind::Interrupt => "sigint",
        };
        Self {
            _kind: kind,
            name: name.to_string(),
        }
    }
}

#[cfg(not(target_family = "unix"))]
impl ShutdownSignal for UnixSignal {
    fn wait(&mut self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        // 非 Unix 平台不支持，直接等待
        Box::pin(std::future::pending::<()>())
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_signal_new() {
        let signal = UnixSignal::new(UnixSignalKind::Terminate);
        assert_eq!(signal.name(), "sigterm");

        let signal = UnixSignal::new(UnixSignalKind::Interrupt);
        assert_eq!(signal.name(), "sigint");
    }
}
