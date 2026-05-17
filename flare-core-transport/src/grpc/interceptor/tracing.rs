use super::extract_trace_info;
use tonic::{Request, Status};
use tracing::debug;

/// 追踪拦截器
pub struct TracingInterceptor;

impl TracingInterceptor {
    pub fn new() -> Self {
        Self
    }

    pub fn intercept<T>(&self, req: Request<T>) -> Result<Request<T>, Status> {
        if let Some(trace_info) = extract_trace_info(&req) {
            debug!(
                trace_id = %trace_info.trace_id,
                request_id = %trace_info.request_id,
                "Processing gRPC request"
            );
        }
        Ok(req)
    }
}

impl Default for TracingInterceptor {
    fn default() -> Self {
        Self::new()
    }
}
