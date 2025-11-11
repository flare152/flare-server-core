//! 工具函数模块

use tonic::{Request, Status};

/// 提取用户ID
pub fn extract_user_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 提取追踪ID
pub fn extract_trace_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 提取请求ID
pub fn extract_request_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 提取设备ID
pub fn extract_device_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-device-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 创建追踪元数据
pub fn create_traced_metadata(
    trace_id: &str,
    request_id: &str,
) -> Result<tonic::metadata::MetadataMap, Status> {
    let mut metadata = tonic::metadata::MetadataMap::new();

    metadata.insert(
        "x-trace-id",
        trace_id
            .parse()
            .map_err(|_| Status::internal("Invalid trace_id"))?,
    );

    metadata.insert(
        "x-request-id",
        request_id
            .parse()
            .map_err(|_| Status::internal("Invalid request_id"))?,
    );

    Ok(metadata)
}

/// 错误转换为 Status
pub fn error_to_status(error: impl std::error::Error, code: tonic::Code) -> Status {
    Status::new(code, error.to_string())
}
