//! gRPC 客户端拦截器
//!
//! 提供自动添加 Context 的拦截器，统一处理所有上下文信息。

use tonic::{
    service::Interceptor,
    Request, Status,
};
use uuid::Uuid;
use crate::context::{
    Context, TenantContext, TraceContext,
};
use super::metadata_codec::{encode_context_to_metadata, decode_context_from_metadata};

/// 客户端上下文拦截器配置
#[derive(Debug, Clone)]
pub struct ClientContextConfig {
    /// 是否自动生成 request_id（默认 true）
    pub auto_generate_request_id: bool,
    /// 是否自动生成 trace_id（默认 true）
    pub auto_generate_trace_id: bool,
    /// 默认的租户上下文（如果提供，会自动添加到所有请求）
    pub default_tenant: Option<TenantContext>,
    /// 默认的 Context（如果提供，会自动合并到所有请求）
    pub default_context: Option<Context>,
}

impl Default for ClientContextConfig {
    fn default() -> Self {
        Self {
            auto_generate_request_id: true,
            auto_generate_trace_id: true,
            default_tenant: None,
            default_context: None,
        }
    }
}

impl ClientContextConfig {
    /// 创建新的客户端上下文配置
    pub fn new() -> Self {
        Self::default()
    }
    
    /// 设置默认租户
    pub fn with_default_tenant(mut self, tenant: TenantContext) -> Self {
        self.default_tenant = Some(tenant);
        self
    }
    
    /// 设置默认 Context
    pub fn with_default_context(mut self, ctx: Context) -> Self {
        self.default_context = Some(ctx);
        self
    }
    
    /// 禁用自动生成 request_id
    pub fn disable_auto_request_id(mut self) -> Self {
        self.auto_generate_request_id = false;
        self
    }
    
    /// 禁用自动生成 trace_id
    pub fn disable_auto_trace_id(mut self) -> Self {
        self.auto_generate_trace_id = false;
        self
    }
}

/// 客户端上下文拦截器
///
/// 自动为所有请求添加 Context（包含租户、请求、追踪等信息）
///
/// # 使用示例
///
/// ```rust,no_run
/// use flare_server_core::client::{ClientContextInterceptor, ClientContextConfig};
/// use flare_server_core::context::TenantContext;
/// use tonic::transport::Channel;
///
/// let config = ClientContextConfig::new()
///     .with_default_tenant(TenantContext::new("tenant-123"));
///
/// let interceptor = ClientContextInterceptor::new(config);
///
/// // 创建客户端并应用拦截器
/// let client = YourServiceClient::with_interceptor(channel, interceptor);
/// ```
#[derive(Clone)]
pub struct ClientContextInterceptor {
    config: ClientContextConfig,
}

impl ClientContextInterceptor {
    /// 创建新的客户端上下文拦截器
    pub fn new(config: ClientContextConfig) -> Self {
        Self { config }
    }
    
    /// 创建默认配置的拦截器（自动生成 request_id 和 trace_id）
    pub fn default() -> Self {
        Self::new(ClientContextConfig::default())
    }
}

impl Interceptor for ClientContextInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        // 1. 尝试从现有 metadata 中提取 Context（如果存在）
        let mut ctx = decode_context_from_metadata(request.metadata())
            .unwrap_or_else(|| Context::root());

        // 2. 合并默认 Context（如果提供）
        if let Some(ref default_ctx) = self.config.default_context {
            // 合并租户
            if ctx.tenant().is_none() {
                if let Some(tenant) = default_ctx.tenant() {
                    ctx = ctx.with_tenant(tenant.clone());
                }
            }
            // 合并请求上下文
            if ctx.request().is_none() {
                if let Some(req_ctx) = default_ctx.request() {
                    ctx = ctx.with_request(req_ctx.clone());
                }
            }
            // 合并追踪
            if ctx.trace().is_none() {
                if let Some(trace) = default_ctx.trace() {
                    ctx = ctx.with_trace(trace.clone());
                }
            }
            // 合并操作者
            if ctx.actor().is_none() {
                if let Some(actor) = default_ctx.actor() {
                    ctx = ctx.with_actor(actor.clone());
                }
            }
            // 合并设备
            if ctx.device().is_none() {
                if let Some(device) = default_ctx.device() {
                    ctx = ctx.with_device(device.clone());
                }
            }
            // 合并其他字段
            if ctx.user_id().is_none() {
                if let Some(user_id) = default_ctx.user_id() {
                    ctx = ctx.with_user_id(user_id);
                }
            }
            if ctx.tenant_id().is_none() {
                if let Some(tenant_id) = default_ctx.tenant_id() {
                    ctx = ctx.with_tenant_id(tenant_id);
                }
            }
            if ctx.session_id().is_none() {
                if let Some(session_id) = default_ctx.session_id() {
                    ctx = ctx.with_session_id(session_id);
                }
            }
        }

        // 3. 应用默认租户（如果提供且未设置）
        if let Some(ref tenant) = self.config.default_tenant {
            if ctx.tenant().is_none() {
                ctx = ctx.with_tenant(tenant.clone());
            }
        }

        // 4. 自动生成 request_id（如果需要且未设置）
        if self.config.auto_generate_request_id && ctx.request_id().is_empty() {
            let request_id = Uuid::new_v4().to_string();
            // 创建新的 Context 并合并现有字段
            let mut new_ctx = Context::with_request_id(&request_id);
            
            // 恢复其他字段
            if !ctx.trace_id().is_empty() {
                new_ctx = new_ctx.with_trace_id(ctx.trace_id());
            }
            if let Some(user_id) = ctx.user_id() {
                new_ctx = new_ctx.with_user_id(user_id);
            }
            if let Some(tenant_id) = ctx.tenant_id() {
                new_ctx = new_ctx.with_tenant_id(tenant_id);
            }
            if let Some(session_id) = ctx.session_id() {
                new_ctx = new_ctx.with_session_id(session_id);
            }
            
            // 恢复其他上下文
            if let Some(tenant) = ctx.tenant() {
                new_ctx = new_ctx.with_tenant(tenant.clone());
            }
            if let Some(req_ctx) = ctx.request() {
                new_ctx = new_ctx.with_request(req_ctx.clone());
            }
            if let Some(trace) = ctx.trace() {
                new_ctx = new_ctx.with_trace(trace.clone());
            }
            if let Some(actor) = ctx.actor() {
                new_ctx = new_ctx.with_actor(actor.clone());
            }
            if let Some(device) = ctx.device() {
                new_ctx = new_ctx.with_device(device.clone());
            }
            
            ctx = new_ctx;
        }

        // 5. 自动生成 trace_id（如果需要且未设置）
        if self.config.auto_generate_trace_id && ctx.trace().is_none() {
            let trace = TraceContext::new(Uuid::new_v4().to_string())
                .with_span_id(Uuid::new_v4().to_string());
            ctx = ctx.with_trace(trace);
        }

        // 6. 确保 RequestContext 有 channel（如果存在）
        if let Some(req_ctx) = ctx.request() {
            if req_ctx.channel.is_empty() {
                let mut updated_req_ctx = req_ctx.clone();
                updated_req_ctx.channel = "grpc".to_string();
                ctx = ctx.with_request(updated_req_ctx);
            }
        }

        // 7. 一次性编码所有上下文到 metadata（性能最优）
        encode_context_to_metadata(request.metadata_mut(), &ctx);
        
        Ok(request)
    }
}

/// 便捷函数：创建带默认配置的拦截器
pub fn default_context_interceptor() -> ClientContextInterceptor {
    ClientContextInterceptor::default()
}

/// 便捷函数：创建带租户的拦截器
pub fn context_interceptor_with_tenant(tenant: TenantContext) -> ClientContextInterceptor {
    ClientContextInterceptor::new(
        ClientContextConfig::new()
            .with_default_tenant(tenant)
    )
}

/// 便捷函数：创建带租户和用户ID的拦截器
pub fn context_interceptor_with_tenant_and_user(
    tenant: TenantContext,
    user_id: String,
) -> ClientContextInterceptor {
    let ctx = Context::root()
        .with_tenant(tenant)
        .with_user_id(user_id);
    ClientContextInterceptor::new(
        ClientContextConfig::new()
            .with_default_context(ctx)
    )
}
