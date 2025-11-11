//! 服务注册发现 Trait 定义

use crate::types::ServiceInfo;
use async_trait::async_trait;

/// 服务注册发现 Trait
#[async_trait]
pub trait ServiceRegistryTrait: Send + Sync {
    /// 注册服务
    async fn register(&mut self, service: ServiceInfo) -> Result<(), Box<dyn std::error::Error>>;

    /// 注销服务
    async fn unregister(&mut self, service_id: &str) -> Result<(), Box<dyn std::error::Error>>;

    /// 发现服务（获取指定类型的所有实例）
    async fn discover(
        &self,
        service_type: &str,
    ) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>>;

    /// 获取服务实例（通过实例ID）
    async fn get_service(
        &self,
        service_id: &str,
    ) -> Result<Option<ServiceInfo>, Box<dyn std::error::Error>>;

    /// 获取所有服务类型
    async fn list_service_types(&self) -> Result<Vec<String>, Box<dyn std::error::Error>>;

    /// 获取所有服务实例（所有类型）
    async fn list_all_services(&self) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>>;

    /// 健康检查
    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>>;
}

/// 服务注册发现（类型别名，用于向后兼容）
pub type ServiceRegistry = Box<dyn ServiceRegistryTrait>;
