//! 服务注册发现模块
//!
//! 支持多种服务注册发现后端：Consul、etcd 和 Service Mesh

pub mod consul;
pub mod etcd;
pub mod load_balancer;
pub mod mesh;
pub mod service_manager;
pub mod trait_def;

use crate::config::RegistryConfig as ConfigRegistryConfig;
pub use consul::ConsulRegistry;
pub use etcd::EtcdRegistry;
pub use load_balancer::{LoadBalanceStrategy, LoadBalancer, ServiceSelector};
pub use mesh::MeshRegistry;
pub use service_manager::ServiceManager;
pub use trait_def::{ServiceRegistry, ServiceRegistryTrait};

/// 服务注册发现类型
#[derive(Debug, Clone)]
pub enum RegistryType {
    Etcd,
    Consul,
    Mesh,
}

impl RegistryType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "consul" => RegistryType::Consul,
            "mesh" => RegistryType::Mesh,
            _ => RegistryType::Etcd,
        }
    }
}

/// 创建服务注册发现实例
pub async fn create_registry(
    config: ConfigRegistryConfig,
) -> Result<Box<dyn ServiceRegistryTrait>, Box<dyn std::error::Error>> {
    let registry_type = RegistryType::from_str(&config.registry_type);

    match registry_type {
        RegistryType::Etcd => {
            let registry = EtcdRegistry::new(
                config.endpoints.clone(),
                config.namespace.clone(),
                config.ttl,
            )
            .await?;
            Ok(Box::new(registry))
        }
        RegistryType::Consul => {
            let registry = ConsulRegistry::new(
                config.endpoints.clone(),
                config.namespace.clone(),
                config.ttl,
            )
            .await?;
            Ok(Box::new(registry))
        }
        RegistryType::Mesh => {
            let registry = MeshRegistry::new(config.namespace.clone()).await?;
            Ok(Box::new(registry))
        }
    }
}
