//! gRPC 客户端模块
//!
//! 提供客户端构建器和配置

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
