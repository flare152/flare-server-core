//! Context 与 gRPC 客户端集成
//!
//! 提供从 Context 提取元数据并设置到 gRPC 请求的功能，与 context/core 对齐。

use tonic::Request;

use crate::grpc::utils::encode_context_to_metadata;
use flare_core_base::context::Context;

/// 将 Context 编码到 gRPC 请求的 metadata。
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::context::Context;
/// use flare_server_core::client::set_context_metadata;
/// use tonic::Request;
///
/// let ctx = Context::with_request_id("req-123").with_tenant_id("tenant-456");
/// let mut request = Request::new(());
/// set_context_metadata(&mut request, &ctx);
/// ```
pub fn set_context_metadata<T, C>(request: &mut Request<T>, ctx: &C)
where
    C: AsRef<Context>,
{
    encode_context_to_metadata(request.metadata_mut(), ctx.as_ref());
}

/// 创建并设置 Context 到 metadata 的 gRPC 请求。
pub fn request_with_context<T, C>(payload: T, ctx: &C) -> Request<T>
where
    C: AsRef<Context>,
{
    let mut request = Request::new(payload);
    set_context_metadata(&mut request, ctx);
    request
}
