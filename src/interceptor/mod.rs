//! gRPC 拦截器模块
//!
//! 提供认证、追踪、日志等拦截器功能

pub mod auth;
pub mod composite;
pub mod logging;
pub mod tracing;

pub use auth::AuthInterceptor;
pub use composite::CompositeInterceptor;
pub use logging::LoggingInterceptor;
pub use tracing::TracingInterceptor;

use tonic::Request;

/// 追踪信息
#[derive(Debug, Clone)]
pub struct TraceInfo {
    pub trace_id: String,
    pub request_id: String,
}

/// 提取追踪信息
pub fn extract_trace_info<T>(req: &Request<T>) -> Option<TraceInfo> {
    let metadata = req.metadata();

    let trace_id = metadata
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let request_id = metadata
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if trace_id.is_empty() && request_id.is_empty() {
        return None;
    }

    Some(TraceInfo {
        trace_id,
        request_id,
    })
}
