use tonic::{Request, Status};
use tracing::info;

/// 日志拦截器
pub struct LoggingInterceptor;

impl LoggingInterceptor {
    pub fn new() -> Self {
        Self
    }

    pub fn intercept<T>(&self, req: Request<T>) -> Result<Request<T>, Status> {
        let _start = std::time::Instant::now();
        let method = req
            .metadata()
            .get(":path")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown");

        info!("gRPC request: {}", method);

        // 注意：这里只是记录请求开始，实际响应日志需要在服务实现中记录
        Ok(req)
    }
}

impl Default for LoggingInterceptor {
    fn default() -> Self {
        Self::new()
    }
}
