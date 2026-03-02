//! Context 与 gRPC 客户端集成
//!
//! 提供从 Context 提取元数据并设置到 gRPC 请求的功能。
//! 这是推荐的客户端上下文处理方式，统一使用 Context 系统。

use tonic::Request;
use crate::context::Context;
use super::metadata_codec::encode_context_to_metadata;

/// 从 Context 设置 gRPC 请求的元数据
///
/// 自动提取 Context 中的所有上下文信息（租户、请求、追踪、操作者、设备等），
/// 并设置到 gRPC 请求的 metadata 中。
///
/// 使用统一的编解码模块，性能最优。
///
/// # 参数
///
/// * `request` - gRPC 请求
/// * `ctx` - Context 对象
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::context::{Context, TenantContext};
/// use flare_server_core::client::set_context_metadata;
/// use tonic::Request;
///
/// let ctx = Context::with_request_id("req-123")
///     .with_tenant(TenantContext::new("tenant-456"));
///
/// let mut request = Request::new(my_request);
/// set_context_metadata(&mut request, &ctx);
/// ```
pub fn set_context_metadata<T>(request: &mut Request<T>, ctx: &Context) {
    encode_context_to_metadata(request.metadata_mut(), ctx);
}

/// 从 Context 创建带元数据的 gRPC 请求
///
/// 便捷函数，用于创建并设置元数据。
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::context::Context;
/// use flare_server_core::client::request_with_context;
///
/// let ctx = Context::root().with_tenant_id("tenant-123");
/// let request = request_with_context(my_request, &ctx);
/// ```
pub fn request_with_context<T>(payload: T, ctx: &Context) -> Request<T> {
    let mut request = Request::new(payload);
    set_context_metadata(&mut request, ctx);
    request
}
