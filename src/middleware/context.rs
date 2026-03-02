//! Context 中间件（统一处理所有上下文）
//!
//! 从 gRPC 请求中统一提取 Context、TenantContext、RequestContext 并注入到请求扩展中。
//! 这是推荐的中间件，替代了原来的 TenantLayer 和 RequestContextLayer。
//!
//! # 设计优势
//!
//! 1. **性能优化**：一次遍历 metadata，统一解码所有上下文信息
//! 2. **代码简洁**：单一中间件，减少重复代码
//! 3. **向后兼容**：同时注入 Context、TenantContext、RequestContext 到 extensions
//! 4. **统一 API**：使用统一的 Context 系统，更符合设计理念
//!
//! # 使用示例
//!
//! ```rust,no_run
//! use flare_server_core::middleware::ContextLayer;
//! use tonic::transport::Server;
//!
//! Server::builder()
//!     .layer(ContextLayer::new().allow_missing())
//!     .add_service(YourServiceServer::new(handler))
//!     .serve(addr)
//!     .await?;
//! ```
//!
//! # 向后兼容
//!
//! 为了保持向后兼容，ContextLayer 会同时注入：
//! - `Context` - 新的统一上下文系统
//! - `TenantContext` - 租户上下文（如果存在）
//! - `RequestContext` - 请求上下文（如果存在）
//!
//! 这样，旧的代码使用 `extract_tenant_context` 和 `extract_request_context` 仍然可以工作。

use std::convert::Infallible;
use std::task::{Context as TaskContext, Poll};
use tower::{Layer, Service};
use tonic::{Request as TonicRequest, Status};
use tonic::body::Body;
use http::Request as HttpRequest;
use tracing::{debug, warn};
use crate::context::{Context, RequestContext, TenantContext};
use crate::client::metadata_codec::decode_context_from_metadata;

/// Context 中间件层（统一处理所有上下文）
#[derive(Clone, Debug)]
pub struct ContextLayer {
    /// 是否允许缺失 Context（默认 false，即必需）
    allow_missing: bool,
    /// 默认租户ID（当请求中未提供时使用）
    default_tenant_id: Option<String>,
}

impl ContextLayer {
    /// 创建新的 Context 中间件层
    pub fn new() -> Self {
        Self {
            allow_missing: false,
            default_tenant_id: None,
        }
    }

    /// 允许缺失 Context（用于内部服务调用等场景）
    pub fn allow_missing(mut self) -> Self {
        self.allow_missing = true;
        self
    }

    /// 设置默认租户ID（当请求中未提供时使用）
    pub fn with_default_tenant_id(mut self, tenant_id: String) -> Self {
        self.default_tenant_id = Some(tenant_id);
        self
    }

    /// 应用 Layer 到 Service（便捷方法）
    /// 
    /// 这个方法封装了 `Layer::layer` 调用，使得可以链式调用
    pub fn layer<S>(&self, service: S) -> <Self as Layer<S>>::Service
    where
        Self: Layer<S>,
    {
        Layer::layer(self, service)
    }
}

impl Default for ContextLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for ContextLayer {
    type Service = ContextService<S>;

    fn layer(&self, service: S) -> Self::Service {
        ContextService {
            inner: service,
            allow_missing: self.allow_missing,
            default_tenant_id: self.default_tenant_id.clone(),
        }
    }
}

/// Context 服务（统一处理所有上下文）
#[derive(Clone, Debug)]
pub struct ContextService<S> {
    inner: S,
    allow_missing: bool,
    default_tenant_id: Option<String>,
}

// 为 ContextService 实现 NamedService trait，以便与 Tonic 的 add_service 兼容
impl<S> tonic::server::NamedService for ContextService<S>
where
    S: tonic::server::NamedService,
{
    const NAME: &'static str = S::NAME;
}

// 注意：不再提供泛型 Service<Request<ReqBody>> 实现
// 因为 Tonic 0.14 只需要 Service<Request<Body>, Error = Infallible>
// 下面的 Body 实现已经满足所有需求

// 为 ContextService 实现 Service<http::Request<Body>, Error = Infallible>，以满足 Tonic 的 add_service 要求
// Tonic 0.14 的 add_service 需要 Service<http::Request<Body>, Error = Infallible> 和 NamedService
// 注意：Tonic 内部会将 http::Request 转换为 tonic::Request，所以我们需要处理 http::Request
impl<S> Service<HttpRequest<Body>> for ContextService<S>
where
    S: Service<HttpRequest<Body>, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = Infallible;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: HttpRequest<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let allow_missing = self.allow_missing;
        let default_tenant_id = self.default_tenant_id.clone();
        
        // 从 http::Request 的 extensions 中提取 Context（如果已经存在）
        // Tonic 会在内部将 http::Request 转换为 tonic::Request，并保留 extensions
        let extensions = req.extensions().clone();
        
        // 从 http::Request 的 headers 构建 MetadataMap（用于解码 Context）
        // 注意：这里我们需要将 http::HeaderMap 转换为 tonic::MetadataMap
        let metadata = {
            use tonic::metadata::MetadataMap;
            let mut metadata = MetadataMap::new();
            for (key, value) in req.headers() {
                if let Ok(key_str) = key.as_str().parse::<tonic::metadata::MetadataKey<_>>() {
                    if let Ok(val_str) = value.to_str() {
                        if let Ok(val) = val_str.parse::<tonic::metadata::MetadataValue<_>>() {
                            metadata.insert(key_str, val);
                        }
                    }
                }
            }
            metadata
        };

        Box::pin(async move {
            let ctx = if let Some(existing_ctx) = extensions.get::<Context>() {
                Some(existing_ctx.clone())
            } else {
                decode_context_from_metadata(&metadata).or_else(|| {
                    extract_context_from_extensions(&extensions)
                })
            };

            match ctx {
                Some(ctx) => {
                    debug!(
                        request_id = %ctx.request_id(),
                        trace_id = %ctx.trace_id(),
                        tenant_id = ctx.tenant_id().unwrap_or("none"),
                        "Extracted Context from request"
                    );

                    req.extensions_mut().insert(ctx.clone());

                    if let Some(tenant) = ctx.tenant() {
                        req.extensions_mut().insert(tenant.clone());
                    } else if let Some(tenant_id) = ctx.tenant_id() {
                        let tenant = TenantContext::new(tenant_id);
                        req.extensions_mut().insert(tenant);
                    } else if let Some(default_id) = &default_tenant_id {
                        let tenant = TenantContext::new(default_id.clone());
                        req.extensions_mut().insert(tenant);
                    }

                    if let Some(req_ctx) = ctx.request() {
                        req.extensions_mut().insert(req_ctx.clone());
                    } else {
                        let req_ctx: RequestContext = (&ctx).into();
                        req.extensions_mut().insert(req_ctx);
                    }

                    inner.call(req).await
                }
                None => {
                    if allow_missing {
                        warn!("Context not found in request, but allow_missing is true");
                        let default_ctx = if let Some(tenant_id) = default_tenant_id {
                            Context::root().with_tenant_id(tenant_id)
                        } else {
                            Context::root()
                        };
                        req.extensions_mut().insert(default_ctx.clone());

                        if let Some(tenant_id) = default_ctx.tenant_id() {
                            let tenant = TenantContext::new(tenant_id);
                            req.extensions_mut().insert(tenant);
                        }
                        let req_ctx: RequestContext = (&default_ctx).into();
                        req.extensions_mut().insert(req_ctx);

                        inner.call(req).await
                    } else {
                        warn!("Context is required but not found in request");
                        // 对于 Error = Infallible，注入默认 Context 并继续处理
                        let default_ctx = Context::root();
                        req.extensions_mut().insert(default_ctx.clone());
                        let req_ctx: RequestContext = (&default_ctx).into();
                        req.extensions_mut().insert(req_ctx);
                        inner.call(req).await
                    }
                }
            }
        })
    }
}

/// 从扩展中提取 Context（兼容性函数）
fn extract_context_from_extensions(extensions: &http::Extensions) -> Option<Context> {
    // 尝试从 RequestContext 和 TenantContext 构建
    if let Some(req_ctx) = extensions.get::<RequestContext>() {
        let mut ctx = Context::from(req_ctx);
        
        if let Some(tenant) = extensions.get::<TenantContext>() {
            ctx = ctx.with_tenant(tenant.clone());
        }
        
        Some(ctx)
    } else {
        None
    }
}

/// 从请求扩展中获取 Context
///
/// 便捷函数，用于在 handler 中获取 Context。
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::middleware::extract_context;
/// use tonic::{Request, Status};
///
/// fn my_handler(req: Request<MyRequest>) -> Result<Response<MyResponse>, Status> {
///     let ctx = extract_context(&req)?;
///     // 使用 ctx...
///     Ok(Response::new(MyResponse {}))
/// }
/// ```
pub fn extract_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<Context, Status> {
    req.extensions()
        .get::<Context>()
        .cloned()
        .ok_or_else(|| Status::internal("Context not found in request extensions"))
}

// ========== 向后兼容的便捷函数 ==========
// 这些函数保留以支持旧代码，它们从 extensions 中提取对应的上下文

/// 从请求中提取租户上下文（便捷函数，向后兼容）
///
/// **注意**：推荐使用 `extract_context` 获取统一的 `Context`。
pub fn extract_tenant_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<&TenantContext> {
    req.extensions().get::<TenantContext>()
}

/// 从请求中提取租户上下文（必需版本，向后兼容）
pub fn require_tenant_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<&TenantContext, Status> {
    extract_tenant_context(req).ok_or_else(|| {
        Status::invalid_argument("TenantContext is required. Please ensure ContextLayer middleware is applied.")
    })
}

/// 从请求中提取租户ID（便捷函数，向后兼容）
pub fn extract_tenant_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<&str> {
    extract_tenant_context(req).map(|t| t.tenant_id.as_str())
}

/// 从请求中提取租户ID（必需版本，向后兼容）
pub fn require_tenant_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<&str, Status> {
    require_tenant_context(req).map(|t| t.tenant_id.as_str())
}

/// 从请求中提取请求上下文（便捷函数，向后兼容）
///
/// **注意**：推荐使用 `extract_context` 获取统一的 `Context`。
pub fn extract_request_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<&RequestContext> {
    req.extensions().get::<RequestContext>()
}

/// 从请求中提取请求上下文（必需版本，向后兼容）
pub fn require_request_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<&RequestContext, Status> {
    extract_request_context(req).ok_or_else(|| {
        Status::invalid_argument("RequestContext is required. Please ensure ContextLayer middleware is applied.")
    })
}

/// 从请求中提取用户ID（便捷函数，从 actor_id 或 x-user-id 中获取）
pub fn extract_user_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<String> {
    // 优先从 RequestContext 中获取
    if let Some(ctx) = extract_request_context(req) {
        if let Some(user_id) = ctx.user_id() {
            return Some(user_id.to_string());
        }
    }
    
    // 兼容性：从 metadata 中获取
    req.metadata()
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 从请求中提取用户ID（必需版本）
pub fn require_user_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<String, Status> {
    extract_user_id(req).ok_or_else(|| {
        Status::invalid_argument("User ID is required. Please provide x-user-id header or set actor in RequestContext.")
    })
}

/// 从请求中提取 actor_id（便捷函数）
pub fn extract_actor_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<String> {
    extract_request_context(req)
        .and_then(|ctx| ctx.actor.as_ref())
        .map(|actor| actor.actor_id.clone())
        .filter(|id| !id.is_empty())
}

/// 从请求中提取 actor_id（必需版本）
pub fn require_actor_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<String, Status> {
    extract_actor_id(req).ok_or_else(|| {
        Status::invalid_argument("Actor ID is required. Please provide x-actor-id header or set actor in RequestContext.")
    })
}

/// 从请求中提取 request_id（便捷函数）
pub fn extract_request_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<&str> {
    extract_request_context(req).map(|ctx| ctx.request_id.as_str())
}

/// 从请求中提取 request_id（必需版本）
pub fn require_request_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<&str, Status> {
    require_request_context(req).map(|ctx| ctx.request_id.as_str())
}
