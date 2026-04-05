//! 自定义通道信号实现

use super::ShutdownSignal;
use std::pin::Pin;
use tokio::sync::oneshot;

/// 自定义通道信号
///
/// 通过 oneshot 通道接收停机信号
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::signal::ChannelSignal;
/// use tokio::sync::oneshot;
///
/// let (tx, rx) = oneshot::channel();
/// let signal = ChannelSignal::new("custom", rx);
///
/// // 发送停机信号
/// tx.send(()).unwrap();
/// ```
pub struct ChannelSignal {
    name: String,
    rx: Option<oneshot::Receiver<()>>,
}

impl ChannelSignal {
    /// 创建新的通道信号
    ///
    /// # 参数
    ///
    /// * `name` - 信号名称
    /// * `rx` - oneshot 接收器
    pub fn new(name: impl Into<String>, rx: oneshot::Receiver<()>) -> Self {
        Self {
            name: name.into(),
            rx: Some(rx),
        }
    }
}

impl ShutdownSignal for ChannelSignal {
    fn wait(&mut self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        if let Some(rx) = self.rx.take() {
            Box::pin(async move {
                let _ = rx.await;
            })
        } else {
            // 如果已经被消费，直接等待
            Box::pin(std::future::pending::<()>())
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_channel_signal() {
        let (tx, rx) = oneshot::channel();
        let mut signal = ChannelSignal::new("test", rx);

        // 发送信号
        tx.send(()).unwrap();

        // 等待信号
        signal.wait().await;
    }

    #[test]
    fn test_channel_signal_name() {
        let (_tx, rx) = oneshot::channel();
        let signal = ChannelSignal::new("custom-signal", rx);
        assert_eq!(signal.name(), "custom-signal");
    }
}
