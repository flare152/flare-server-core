//! ServiceRegistry trait 定义
//!
//! 服务注册抽象，支持多种服务发现系统：Consul, Etcd, Nacos 等

use crate::error::RegistryError;
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

/// 服务信息
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    /// 服务名称
    pub name: String,
    /// 服务 ID
    pub id: String,
    /// 服务地址
    pub address: SocketAddr,
    /// 服务元数据
    pub metadata: HashMap<String, String>,
    /// 心跳 TTL
    pub ttl: Duration,
}

impl ServiceInfo {
    /// 创建新的服务信息
    pub fn new(name: impl Into<String>, id: impl Into<String>, address: SocketAddr) -> Self {
        Self {
            name: name.into(),
            id: id.into(),
            address,
            metadata: HashMap::new(),
            ttl: Duration::from_secs(10),
        }
    }

    /// 添加元数据
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// 设置心跳 TTL
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }
}

/// 服务注册抽象
///
/// 支持多种服务发现系统：Consul, Etcd, Nacos 等
///
/// # 实现说明
///
/// - 使用 Boxed Future 模式支持 trait object
/// - 所有服务注册器必须实现此 trait
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::registry::{ServiceRegistry, ServiceInfo};
/// use flare_core_runtime::error::RegistryError;
/// use std::pin::Pin;
/// use std::future::Future;
///
/// struct ConsulRegistry {
///     // Consul 客户端
/// }
///
/// impl ServiceRegistry for ConsulRegistry {
///     fn register<'a>(&'a mut self, service: &'a ServiceInfo) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + 'a>> {
///         Box::pin(async move {
///             // 注册服务到 Consul
///             Ok(())
///         })
///     }
///
///     fn deregister<'a>(&'a mut self, service: &'a ServiceInfo) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + 'a>> {
///         Box::pin(async move {
///             // 从 Consul 注销服务
///             Ok(())
///         })
///     }
///
///     fn send_heartbeat<'a>(&'a mut self, service_id: &'a str) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + 'a>> {
///         Box::pin(async move {
///             // 发送心跳
///             Ok(())
///         })
///     }
///
///     fn shutdown(&mut self) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + '_>> {
///         Box::pin(async move {
///             // 关闭注册器
///             Ok(())
///         })
///     }
/// }
/// ```
pub trait ServiceRegistry: Send {
    /// 注册服务
    ///
    /// # 参数
    ///
    /// * `service` - 服务信息
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `RegistryError`
    fn register<'a>(
        &'a mut self,
        service: &'a ServiceInfo,
    ) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + 'a>>;

    /// 注销服务
    ///
    /// # 参数
    ///
    /// * `service` - 服务信息
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `RegistryError`
    fn deregister<'a>(
        &'a mut self,
        service: &'a ServiceInfo,
    ) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + 'a>>;

    /// 发送心跳
    ///
    /// # 参数
    ///
    /// * `service_id` - 服务 ID
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `RegistryError`
    fn send_heartbeat<'a>(
        &'a mut self,
        service_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + 'a>>;

    /// 关闭注册器
    ///
    /// # 返回
    ///
    /// 成功返回 `Ok(())`，失败返回 `RegistryError`
    fn shutdown(&mut self) -> Pin<Box<dyn Future<Output = Result<(), RegistryError>> + Send + '_>>;
}
