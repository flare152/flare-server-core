//! Context 工具函数 (gRPC 相关)

use flare_core_base::context::Ctx;
use tonic::{Request, Status};

use crate::grpc::middleware::context::get_context;
use crate::grpc::utils::metadata_codec::decode_context_from_metadata;

// -----------------------------------------------------------------------------
// gRPC Request 提取函数
// -----------------------------------------------------------------------------

/// 合并 extensions（ContextLayer 注入）与 metadata 解码结果，优先保留非空的 user/tenant。
fn merge_request_context<T>(req: &Request<T>) -> Option<Ctx> {
    let from_extensions = get_context(req).cloned();
    let from_metadata = decode_context_from_metadata(req.metadata());

    match (from_extensions, from_metadata) {
        (Some(ext), Some(md)) => {
            let mut merged = (*ext).clone();
            if merged.user_id().is_none_or(|u| u.is_empty()) {
                if let Some(uid) = md.user_id().filter(|u| !u.is_empty()) {
                    merged = merged.with_user_id(uid);
                }
            }
            if merged.tenant_id().is_none_or(|t| t.is_empty()) {
                if let Some(tid) = md.tenant_id().filter(|t| !t.is_empty()) {
                    merged = merged.with_tenant_id(tid);
                }
            }
            if merged.trace_id().is_empty() {
                let trace = md.trace_id();
                if !trace.is_empty() {
                    merged = merged.with_trace_id(trace);
                }
            }
            if merged.device_id().is_none_or(|d| d.is_empty()) {
                if let Some(device) = md.device_id().filter(|d| !d.is_empty()) {
                    merged = merged.with_device_id(device);
                }
            }
            Some(std::sync::Arc::new(merged))
        }
        (Some(ext), None) => Some(ext),
        (None, Some(md)) => Some(md),
        (None, None) => None,
    }
}

/// 从 gRPC Request 提取 Ctx（必需版本）
///
/// 优先使用 ContextLayer 注入的 extensions，并与 metadata 合并。
pub fn require_ctx_from_request<T>(req: &Request<T>) -> Result<Ctx, Status> {
    merge_request_context(req)
        .ok_or_else(|| Status::internal("Context not found in request metadata"))
}

/// 从 gRPC Request 提取 Ctx（可选版本）
///
/// 如果 Request 中没有 Context 信息,返回 None。
pub fn extract_ctx_from_request_opt<T>(req: &Request<T>) -> Option<Ctx> {
    merge_request_context(req)
}

/// 从 gRPC Request 中提取租户ID（便捷函数,必需版本）
///
/// 自动从 Context 中提取租户ID。
pub fn require_tenant_id<T>(req: &Request<T>) -> Result<String, Status> {
    let ctx = require_ctx_from_request(req)?;
    ctx.tenant_id()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| Status::invalid_argument("Tenant ID is required in context"))
}

/// 从 gRPC Request 中提取用户ID（便捷函数,必需版本）
///
/// 自动从 Context 中提取用户ID。
pub fn require_user_id<T>(req: &Request<T>) -> Result<String, Status> {
    let ctx = require_ctx_from_request(req)?;
    if let Some(actor) = ctx.actor()
        && !actor.actor_id().is_empty()
    {
        return Ok(actor.actor_id().to_string());
    }
    ctx.user_id()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| Status::invalid_argument("User ID is required in context"))
}

/// 从 gRPC Request 中提取会话ID（便捷函数,可选版本）
pub fn extract_session_id<T>(req: &Request<T>) -> Option<String> {
    let ctx = extract_ctx_from_request_opt(req)?;
    ctx.session_id().map(|s| s.to_string())
}

/// 从 gRPC Request 中提取请求ID（便捷函数,必需版本）
pub fn require_request_id<T>(req: &Request<T>) -> Result<String, Status> {
    let ctx = require_ctx_from_request(req)?;
    let request_id = ctx.request_id();
    if request_id.is_empty() {
        return Err(Status::invalid_argument(
            "Request ID is required in context",
        ));
    }
    Ok(request_id.to_string())
}

/// 从 Context 中提取用户ID
pub fn extract_user_id(ctx: &Ctx) -> Option<String> {
    ctx.user_id()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 从 Context 中提取 TraceID
pub fn extract_trace_id(ctx: &Ctx) -> String {
    ctx.trace_id().to_string()
}

/// 从 Context 中提取设备ID
pub fn extract_device_id(ctx: &Ctx) -> Option<String> {
    ctx.device_id()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 创建带追踪信息的 Metadata
pub fn create_traced_metadata(trace_id: &str) -> tonic::metadata::MetadataMap {
    let mut metadata = tonic::metadata::MetadataMap::new();
    metadata.insert("x-trace-id", trace_id.parse().unwrap());
    metadata
}

/// 将错误转换为 gRPC Status
pub fn error_to_status(err: impl std::fmt::Display) -> Status {
    Status::internal(err.to_string())
}

/// 等待服务器就绪
pub async fn wait_for_server_ready(_channel: tonic::transport::Channel) -> Result<(), Status> {
    // tonic::transport::Channel 会自动重连,无需手动等待
    Ok(())
}
