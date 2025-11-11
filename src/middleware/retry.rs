//! 重试中间件
//!
//! 注意：重试中间件主要用于客户端，服务端通常不需要重试

use tower::Layer;

/// 重试中间件层
///
/// 注意：此中间件主要用于客户端场景
/// 服务端通常不需要重试，因为重试应该由客户端处理
#[derive(Clone)]
pub struct RetryLayer {
    max_retries: usize,
}

impl RetryLayer {
    pub fn new(max_retries: usize) -> Self {
        Self { max_retries }
    }
}

// 注意：tower 的重试中间件需要特定的 Service trait 实现
// 这里提供一个简化的实现，实际使用时需要根据具体场景调整
// 注意：这里简化了实现，直接返回服务，不进行实际的 Service trait 约束
impl<S> Layer<S> for RetryLayer
where
    S: Clone + Send + 'static,
{
    type Service = S;

    fn layer(&self, service: S) -> Self::Service {
        // 简化实现：直接返回服务
        // 实际的重试逻辑应该在客户端实现
        let _ = self.max_retries;
        service
    }
}
