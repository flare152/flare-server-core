use std::time::Duration;
use tower::Layer;

/// 超时中间件层
#[derive(Clone)]
pub struct TimeoutLayer {
    timeout: Duration,
}

impl TimeoutLayer {
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl<S> Layer<S> for TimeoutLayer {
    type Service = tower::timeout::Timeout<S>;

    fn layer(&self, service: S) -> Self::Service {
        tower::timeout::Timeout::new(service, self.timeout)
    }
}
