//! 服务发现后端抽象和实现

pub mod etcd;
pub mod consul;
pub mod dns;
pub mod mesh;

use async_trait::async_trait;
use crate::discovery::instance::ServiceInstance;

/// 服务发现后端 trait
///
/// 所有服务发现后端（etcd、consul、DNS、Mesh）都需要实现这个 trait
/// 注意：由于需要动态分发（dyn），使用 async-trait
#[async_trait]
pub trait DiscoveryBackend: Send + Sync {
    /// 发现服务实例
    ///
    /// # 参数
    /// * `service_type` - 服务类型
    /// * `namespace` - 命名空间（可选）
    /// * `version` - 版本（可选）
    /// * `tags` - 标签过滤器（可选）
    ///
    /// # 返回
    /// 返回服务实例列表
    async fn discover(
        &self,
        service_type: &str,
        namespace: Option<&str>,
        version: Option<&str>,
        tags: Option<&std::collections::HashMap<String, String>>,
    ) -> Result<Vec<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>>;

    /// 注册服务实例
    async fn register(&self, instance: ServiceInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 注销服务实例
    async fn unregister(&self, instance_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// 监听服务变化
    async fn watch(
        &self,
        service_type: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>>;

    /// 发送心跳/更新 TTL（用于保持服务健康状态）
    ///
    /// 不同后端有不同的实现方式：
    /// - **etcd**: 重新注册服务以续期 lease TTL
    /// - **Consul**: 调用 TTL 更新 API (`/v1/agent/check/pass/:check_id`)
    /// - **DNS/Mesh**: 可能不需要心跳，提供默认实现
    ///
    /// # 参数
    /// * `instance` - 服务实例
    ///
    /// # 返回
    /// * `Ok(())` - 心跳发送成功
    /// * `Err` - 心跳发送失败
    ///
    /// # 默认实现
    /// 默认实现是重新注册服务（适用于 etcd 等后端）
    async fn heartbeat(&self, instance: &ServiceInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 默认实现：重新注册服务以续期 TTL
        self.register(instance.clone()).await
    }
}

// BackendType 从 config 模块导出，不需要在这里重复导出
