//! HTTP 服务适配器
//!
//! 实现 Task trait,使 HTTP 服务可以被 ServiceRuntime 管理

use flare_core_runtime::task::{Task, TaskResult};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

/// HTTP 服务适配器
///
/// 包装 HTTP Server (axum/actix-web等),实现 Task trait
///
/// # 示例
///
/// ```rust
/// use flare_core_transport::http::HttpAdapter;
/// use axum::Router;
///
/// let adapter = HttpAdapter::new("my-http", "0.0.0.0:8080".parse().unwrap(), |shutdown_rx| {
///     async move {
///         // 启动 axum server
///         Ok(())
///     }
/// });
/// ```
pub struct HttpAdapter<F, Fut>
where
    F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    name: String,
    address: SocketAddr,
    dependencies: Vec<String>,
    critical: bool,
    serve_fn: Option<F>,
    _marker: std::marker::PhantomData<Fut>,
}

impl<F, Fut> HttpAdapter<F, Fut>
where
    F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    /// 创建新的 HTTP 适配器
    ///
    /// # 参数
    ///
    /// * `name` - 服务名称
    /// * `address` - 监听地址
    /// * `serve_fn` - 服务启动函数,接收 shutdown_rx
    pub fn new(name: impl Into<String>, address: SocketAddr, serve_fn: F) -> Self {
        Self {
            name: name.into(),
            address,
            dependencies: Vec::new(),
            critical: true, // HTTP 服务默认为关键任务
            serve_fn: Some(serve_fn),
            _marker: std::marker::PhantomData,
        }
    }

    /// 设置依赖
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    /// 设置是否为关键任务
    pub fn with_critical(mut self, critical: bool) -> Self {
        self.critical = critical;
        self
    }

    /// 获取监听地址
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

impl<F, Fut> Task for HttpAdapter<F, Fut>
where
    F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn dependencies(&self) -> Vec<String> {
        self.dependencies.clone()
    }

    fn run(
        self: Box<Self>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
        if let Some(serve_fn) = self.serve_fn {
            Box::pin(serve_fn(shutdown_rx))
        } else {
            Box::pin(async { Err("serve_fn already consumed".into()) })
        }
    }

    fn is_critical(&self) -> bool {
        self.critical
    }
}

/// 简化的 HTTP 适配器构建器
pub struct HttpAdapterBuilder {
    name: String,
    address: SocketAddr,
    dependencies: Vec<String>,
    critical: bool,
}

impl HttpAdapterBuilder {
    /// 创建新的构建器
    pub fn new(name: impl Into<String>, address: SocketAddr) -> Self {
        Self {
            name: name.into(),
            address,
            dependencies: Vec::new(),
            critical: true,
        }
    }

    /// 设置依赖
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }

    /// 设置是否为关键任务
    pub fn with_critical(mut self, critical: bool) -> Self {
        self.critical = critical;
        self
    }

    /// 构建 HTTP 适配器
    pub fn build<F, Fut>(self, serve_fn: F) -> HttpAdapter<F, Fut>
    where
        F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
    {
        HttpAdapter {
            name: self.name,
            address: self.address,
            dependencies: self.dependencies,
            critical: self.critical,
            serve_fn: Some(serve_fn),
            _marker: std::marker::PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_adapter_new() {
        let adapter = HttpAdapter::new(
            "test-http",
            "0.0.0.0:8080".parse().unwrap(),
            |_shutdown_rx| async { Ok(()) },
        );

        assert_eq!(adapter.name(), "test-http");
        assert!(adapter.is_critical());
    }

    #[test]
    fn test_http_adapter_builder() {
        let adapter = HttpAdapterBuilder::new("test-http", "0.0.0.0:8080".parse().unwrap())
            .with_dependencies(vec!["db".to_string()])
            .with_critical(false)
            .build(|_shutdown_rx| async { Ok(()) });

        assert_eq!(adapter.name(), "test-http");
        assert_eq!(adapter.dependencies(), vec!["db"]);
        assert!(!adapter.is_critical());
    }
}
