//! Context 中间件
//!
//! 从 gRPC 请求 metadata 解码并注入 `Context`（Ctx）到请求扩展，与 context/core 对齐。

use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tonic::body::Body;
use tonic::{Request as TonicRequest, Status};
use tower::{Layer, Service};
use tracing::{debug, warn};

use crate::client::metadata_codec::decode_context_from_metadata;
use flare_core_base::context::{Context, Ctx};
use http::Request as HttpRequest;

/// Context 中间件层
#[derive(Clone, Debug)]
pub struct ContextLayer {
    allow_missing: bool,
    default_tenant_id: Option<String>,
}

impl ContextLayer {
    pub fn new() -> Self {
        Self {
            allow_missing: false,
            default_tenant_id: None,
        }
    }

    pub fn allow_missing(mut self) -> Self {
        self.allow_missing = true;
        self
    }

    pub fn with_default_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.default_tenant_id = Some(tenant_id.into());
        self
    }

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

#[derive(Clone, Debug)]
pub struct ContextService<S> {
    inner: S,
    allow_missing: bool,
    default_tenant_id: Option<String>,
}

impl<S> tonic::server::NamedService for ContextService<S>
where
    S: tonic::server::NamedService,
{
    const NAME: &'static str = S::NAME;
}

impl<S> Service<HttpRequest<Body>> for ContextService<S>
where
    S: Service<HttpRequest<Body>, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: HttpRequest<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let allow_missing = self.allow_missing;
        let default_tenant_id = self.default_tenant_id.clone();
        let metadata = {
            use tonic::metadata::MetadataMap;
            let mut m = MetadataMap::new();
            for (key, value) in req.headers() {
                if let Ok(k) = key.as_str().parse::<tonic::metadata::MetadataKey<_>>() {
                    if let Ok(s) = value.to_str() {
                        if let Ok(v) = s.parse::<tonic::metadata::MetadataValue<_>>() {
                            m.insert(k, v);
                        }
                    }
                }
            }
            m
        };

        Box::pin(async move {
            let ctx_opt = req
                .extensions()
                .get::<Ctx>()
                .map(Arc::clone)
                .or_else(|| decode_context_from_metadata(&metadata));

            let ctx = match ctx_opt {
                Some(arc) => arc,
                None if allow_missing => {
                    warn!("Context not found, allow_missing=true, using default");
                    let default_ctx = match &default_tenant_id {
                        Some(tid) => Context::root().with_tenant_id(tid.as_str()),
                        None => Context::root(),
                    };
                    req.extensions_mut().insert(Arc::new(default_ctx));
                    return inner.call(req).await;
                }
                None => {
                    warn!("Context required but not found, injecting root");
                    let default_ctx = Context::root();
                    req.extensions_mut().insert(Arc::new(default_ctx));
                    return inner.call(req).await;
                }
            };

            debug!(
                request_id = %ctx.request_id(),
                trace_id = %ctx.trace_id(),
                tenant_id = ctx.tenant_id().unwrap_or("none"),
                "Context from request"
            );

            if ctx.tenant_id().is_none() {
                if let Some(tid) = &default_tenant_id {
                    let with_tenant = (*ctx).clone().with_tenant_id(tid.as_str());
                    req.extensions_mut().insert(Arc::new(with_tenant));
                } else {
                    req.extensions_mut().insert(ctx);
                }
            } else {
                req.extensions_mut().insert(ctx);
            }

            inner.call(req).await
        })
    }
}

/// 从请求扩展中取出 Context
pub fn extract_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<Context, Status> {
    req.extensions()
        .get::<Ctx>()
        .map(|arc| (**arc).clone())
        .ok_or_else(|| Status::internal("Context not found in request extensions"))
}

/// 从请求中取 Ctx 引用（避免 clone）
pub fn get_context<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<&Ctx> {
    req.extensions().get::<Ctx>()
}

/// 租户 ID
pub fn extract_tenant_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<String> {
    get_context(req).and_then(|c| c.tenant_id().map(String::from))
}

pub fn require_tenant_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<String, Status> {
    extract_tenant_id(req)
        .ok_or_else(|| Status::invalid_argument("tenant_id required (ContextLayer + x-tenant-id)"))
}

/// 用户 ID（user_id 或 actor.actor_id）
pub fn extract_user_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<String> {
    get_context(req).and_then(|c| {
        c.user_id()
            .map(String::from)
            .or_else(|| c.actor().map(|a| a.actor_id().to_string()))
    })
}

pub fn require_user_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<String, Status> {
    extract_user_id(req)
        .ok_or_else(|| Status::invalid_argument("user_id required (x-user-id or actor)"))
}

/// request_id
pub fn extract_request_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<String> {
    get_context(req).map(|c| c.request_id().to_string())
}

pub fn require_request_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<String, Status> {
    extract_request_id(req).ok_or_else(|| Status::invalid_argument("request_id required"))
}

/// actor_id（来自 Context 的 actor）
pub fn extract_actor_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Option<String> {
    get_context(req)
        .and_then(|c| c.actor().map(|a| a.actor_id().to_string()))
        .filter(|s| !s.is_empty())
}

pub fn require_actor_id<ReqBody>(req: &TonicRequest<ReqBody>) -> Result<String, Status> {
    extract_actor_id(req).ok_or_else(|| {
        Status::invalid_argument("actor_id required (x-actor-id or actor in Context)")
    })
}
