//! gRPC 服务端模块
//!
//! 提供服务器构建器和配置

use crate::error::Result;
use std::net::SocketAddr;
use tonic::transport::Server;

/// 服务端配置
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub addr: SocketAddr,
    pub max_concurrent_streams: Option<u32>,
    pub tcp_nodelay: bool,
    pub tcp_keepalive: Option<std::time::Duration>,
}

/// 服务器构建器
pub struct ServerBuilder {
    config: ServerConfig,
}

impl ServerBuilder {
    pub fn new() -> Self {
        Self {
            config: ServerConfig {
                addr: "127.0.0.1:50051".parse().unwrap(),
                max_concurrent_streams: Some(1000),
                tcp_nodelay: true,
                tcp_keepalive: Some(std::time::Duration::from_secs(60)),
            },
        }
    }

    pub fn addr(mut self, addr: SocketAddr) -> Self {
        self.config.addr = addr;
        self
    }

    pub fn max_concurrent_streams(mut self, max: u32) -> Self {
        self.config.max_concurrent_streams = Some(max);
        self
    }

    pub fn tcp_nodelay(mut self, nodelay: bool) -> Self {
        self.config.tcp_nodelay = nodelay;
        self
    }

    pub fn tcp_keepalive(mut self, keepalive: std::time::Duration) -> Self {
        self.config.tcp_keepalive = Some(keepalive);
        self
    }

    pub fn build(self) -> Result<Server> {
        let mut server = Server::builder();

        if let Some(max_streams) = self.config.max_concurrent_streams {
            server = server.concurrency_limit_per_connection(max_streams as usize);
        }

        Ok(server)
    }

    pub fn get_addr(&self) -> SocketAddr {
        self.config.addr
    }
}

impl Default for ServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// gRPC 服务器
pub struct GrpcServer {
    config: ServerConfig,
}

impl GrpcServer {
    pub fn new(config: ServerConfig) -> Self {
        Self { config }
    }

    pub fn builder(&self) -> Server {
        let mut server = Server::builder();

        if let Some(max_streams) = self.config.max_concurrent_streams {
            server = server.concurrency_limit_per_connection(max_streams as usize);
        }

        server
    }

    pub fn get_addr(&self) -> SocketAddr {
        self.config.addr
    }
}
