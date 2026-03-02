//! gRPC 客户端模块
//!
//! 提供客户端构建器、Context 集成和拦截器
//!
//! # 设计理念
//!
//! 统一使用 `Context` 系统处理所有上下文信息，通过 `metadata_codec` 统一编解码，
//! 性能最优，代码最简洁。
//!
//! # 使用示例
//!
//! ## 方式 1：使用拦截器（推荐）
//!
//! ```rust,no_run
//! use flare_server_core::client::{ClientContextInterceptor, ClientContextConfig};
//! use flare_server_core::context::TenantContext;
//!
//! let config = ClientContextConfig::new()
//!     .with_default_tenant(TenantContext::new("tenant-123"));
//! let interceptor = ClientContextInterceptor::new(config);
//! let client = YourServiceClient::with_interceptor(channel, interceptor);
//! ```
//!
//! ## 方式 2：手动设置 Context
//!
//! ```rust,no_run
//! use flare_server_core::client::set_context_metadata;
//! use flare_server_core::context::Context;
//!
//! let ctx = Context::root().with_tenant_id("tenant-123");
//! let mut request = Request::new(my_request);
//! set_context_metadata(&mut request, &ctx);
//! ```

pub mod metadata_codec;
pub mod interceptor;
pub mod context;

// 核心编解码功能
pub use metadata_codec::{
    encode_context_to_metadata, decode_context_from_metadata,
};

// 拦截器
pub use interceptor::{
    ClientContextInterceptor, ClientContextConfig,
    default_context_interceptor, context_interceptor_with_tenant,
    context_interceptor_with_tenant_and_user,
};

// Context 集成
pub use context::{
    set_context_metadata, request_with_context,
};

use crate::error::Result;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

/// 客户端配置
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub address: String,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub max_retries: usize,
    pub tls_enabled: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            address: "http://localhost:50051".to_string(),
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(30),
            max_retries: 3,
            tls_enabled: false,
        }
    }
}

/// 客户端构建器
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }

    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.config.address = address.into();
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    pub fn max_retries(mut self, retries: usize) -> Self {
        self.config.max_retries = retries;
        self
    }

    pub async fn build(self) -> Result<Channel> {
        let mut endpoint = Endpoint::from_shared(self.config.address.clone()).map_err(|e| {
            crate::error::FlareError::connection_failed(format!("Invalid address: {}", e))
        })?;

        endpoint = endpoint
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.timeout);

        let channel = endpoint.connect().await.map_err(|e| {
            crate::error::FlareError::connection_failed(format!("Failed to connect: {}", e))
        })?;

        Ok(channel)
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// gRPC 客户端
pub struct GrpcClient {
    channel: Channel,
    config: ClientConfig,
}

impl GrpcClient {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let channel = ClientBuilder::new()
            .address(config.address.clone())
            .connect_timeout(config.connect_timeout)
            .timeout(config.timeout)
            .build()
            .await?;

        Ok(Self { channel, config })
    }

    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }

    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
}
