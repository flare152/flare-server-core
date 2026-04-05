//! gRPC 客户端拦截器
//!
//! 提供自动添加 Context 的拦截器，与 context/core 对齐。

use tonic::service::Interceptor;
use tonic::{Request, Status};
use uuid::Uuid;

use crate::grpc::utils::{decode_context_from_metadata, encode_context_to_metadata};
use flare_core_base::context::Context;

/// 客户端上下文拦截器配置
#[derive(Debug, Clone)]
pub struct ClientContextConfig {
    /// 是否自动生成 request_id（默认 true）
    pub auto_generate_request_id: bool,
    /// 是否自动生成 trace_id（默认 true）
    pub auto_generate_trace_id: bool,
    /// 默认 tenant_id（如果提供，会自动添加到所有请求）
    pub default_tenant_id: Option<String>,
    /// 默认 Context（如果提供，会合并到所有请求）
    pub default_context: Option<Context>,
}

impl Default for ClientContextConfig {
    fn default() -> Self {
        Self {
            auto_generate_request_id: true,
            auto_generate_trace_id: true,
            default_tenant_id: None,
            default_context: None,
        }
    }
}

impl ClientContextConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_default_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.default_tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_default_context(mut self, ctx: Context) -> Self {
        self.default_context = Some(ctx);
        self
    }

    pub fn disable_auto_request_id(mut self) -> Self {
        self.auto_generate_request_id = false;
        self
    }

    pub fn disable_auto_trace_id(mut self) -> Self {
        self.auto_generate_trace_id = false;
        self
    }
}

/// 客户端上下文拦截器：自动为所有请求添加/合并 Context 并编码到 metadata。
#[derive(Clone)]
pub struct ClientContextInterceptor {
    config: ClientContextConfig,
}

impl ClientContextInterceptor {
    pub fn new(config: ClientContextConfig) -> Self {
        Self { config }
    }

    pub fn default() -> Self {
        Self::new(ClientContextConfig::default())
    }
}

impl Interceptor for ClientContextInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let mut ctx = decode_context_from_metadata(request.metadata())
            .map(|arc| (*arc).clone())
            .unwrap_or_else(Context::root);

        if let Some(ref default) = self.config.default_context {
            if ctx.tenant_id().is_none() {
                if let Some(tid) = default.tenant_id() {
                    ctx = ctx.with_tenant_id(tid);
                }
            }
            if ctx.trace_id().is_empty() {
                let tid = default.trace_id();
                if !tid.is_empty() {
                    ctx = ctx.with_trace_id(tid);
                }
            }
            if ctx.user_id().is_none() {
                if let Some(uid) = default.user_id() {
                    ctx = ctx.with_user_id(uid);
                }
            }
            if ctx.device_id().is_none() {
                if let Some(did) = default.device_id() {
                    ctx = ctx.with_device_id(did);
                }
            }
            if ctx.platform().is_none() {
                if let Some(p) = default.platform() {
                    ctx = ctx.with_platform(p);
                }
            }
            if ctx.session_id().is_none() {
                if let Some(sid) = default.session_id() {
                    ctx = ctx.with_session_id(sid);
                }
            }
            if ctx.actor().is_none() {
                if let Some(actor) = default.actor() {
                    ctx = ctx.with_actor(actor.clone());
                }
            }
        }

        if let Some(ref tenant_id) = self.config.default_tenant_id {
            if ctx.tenant_id().is_none() {
                ctx = ctx.with_tenant_id(tenant_id.as_str());
            }
        }

        if self.config.auto_generate_request_id && ctx.request_id().is_empty() {
            let old = ctx;
            ctx = Context::with_request_id(Uuid::new_v4().to_string())
                .with_trace_id(old.trace_id())
                .with_tenant_id(old.tenant_id().unwrap_or(""))
                .with_user_id(old.user_id().unwrap_or(""));
            if let Some(sid) = old.session_id() {
                ctx = ctx.with_session_id(sid);
            }
            if let Some(did) = old.device_id() {
                ctx = ctx.with_device_id(did);
            }
            if let Some(p) = old.platform() {
                ctx = ctx.with_platform(p);
            }
            if let Some(actor) = old.actor() {
                ctx = ctx.with_actor(actor.clone());
            }
        }

        if self.config.auto_generate_trace_id && ctx.trace_id().is_empty() {
            ctx = ctx.with_trace_id(Uuid::new_v4().to_string());
        }

        encode_context_to_metadata(request.metadata_mut(), &ctx);
        Ok(request)
    }
}

/// 创建带默认配置的拦截器
pub fn default_context_interceptor() -> ClientContextInterceptor {
    ClientContextInterceptor::default()
}

/// 创建带默认 tenant_id 的拦截器
pub fn context_interceptor_with_tenant(tenant_id: impl Into<String>) -> ClientContextInterceptor {
    ClientContextInterceptor::new(ClientContextConfig::new().with_default_tenant_id(tenant_id))
}

/// 创建带默认 tenant_id 和 user_id 的拦截器
pub fn context_interceptor_with_tenant_and_user(
    tenant_id: impl AsRef<str>,
    user_id: impl AsRef<str>,
) -> ClientContextInterceptor {
    let ctx = Context::root()
        .with_tenant_id(tenant_id.as_ref())
        .with_user_id(user_id.as_ref());
    ClientContextInterceptor::new(ClientContextConfig::new().with_default_context(ctx))
}
