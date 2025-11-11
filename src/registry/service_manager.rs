//! 服务管理器
//!
//! 提供高级服务管理功能，包括服务发现、负载均衡、健康检查等

use super::{LoadBalanceStrategy, ServiceRegistryTrait, ServiceSelector};
use crate::types::ServiceInfo;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 服务管理器
///
/// 封装服务注册发现和负载均衡，提供便捷的服务管理接口
pub struct ServiceManager {
    registry: Arc<RwLock<Box<dyn ServiceRegistryTrait>>>,
    selector: ServiceSelector,
    cache: Arc<RwLock<std::collections::HashMap<String, Vec<ServiceInfo>>>>,
    cache_ttl: std::time::Duration,
    last_update: Arc<RwLock<std::collections::HashMap<String, std::time::Instant>>>,
}

impl ServiceManager {
    /// 创建新的服务管理器
    pub fn new(registry: Box<dyn ServiceRegistryTrait>, strategy: LoadBalanceStrategy) -> Self {
        Self {
            registry: Arc::new(RwLock::new(registry)),
            selector: ServiceSelector::new(strategy),
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            cache_ttl: std::time::Duration::from_secs(30),
            last_update: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// 设置缓存 TTL
    pub fn set_cache_ttl(&mut self, ttl: std::time::Duration) {
        self.cache_ttl = ttl;
    }

    /// 获取服务实例（带缓存和负载均衡）
    pub async fn get_service_instance(
        &self,
        service_type: &str,
        key: Option<&str>,
    ) -> Result<Option<ServiceInfo>, Box<dyn std::error::Error>> {
        let services = self.get_services(service_type).await?;
        Ok(self.selector.select_service(&services, key).await)
    }

    /// 获取服务地址（带缓存和负载均衡）
    pub async fn get_service_address(
        &self,
        service_type: &str,
        key: Option<&str>,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let services = self.get_services(service_type).await?;
        Ok(self.selector.select_address(&services, key).await)
    }

    /// 获取所有服务实例（带缓存）
    pub async fn get_services(
        &self,
        service_type: &str,
    ) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
        // 检查缓存
        {
            let cache = self.cache.read().await;
            let last_update = self.last_update.read().await;

            if let Some(cached_services) = cache.get(service_type) {
                if let Some(&update_time) = last_update.get(service_type) {
                    if update_time.elapsed() < self.cache_ttl {
                        return Ok(cached_services.clone());
                    }
                }
            }
        }

        // 缓存过期或不存在，从注册中心获取
        let registry = self.registry.read().await;
        let services = registry.discover(service_type).await?;

        // 更新缓存
        {
            let mut cache = self.cache.write().await;
            let mut last_update = self.last_update.write().await;
            cache.insert(service_type.to_string(), services.clone());
            last_update.insert(service_type.to_string(), std::time::Instant::now());
        }

        Ok(services)
    }

    /// 获取所有网关实例
    pub async fn get_gateway_instances(
        &self,
    ) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
        self.get_services("gateway").await
    }

    /// 选择网关实例（用于消息推送）
    ///
    /// 使用一致性哈希，确保同一用户总是路由到同一个网关
    pub async fn select_gateway(
        &self,
        user_id: Option<&str>,
    ) -> Result<Option<ServiceInfo>, Box<dyn std::error::Error>> {
        let key = user_id.map(|id| format!("gateway:{}", id));
        self.get_service_instance("gateway", key.as_deref()).await
    }

    /// 选择网关实例（通过用户ID）
    ///
    /// 便捷方法，直接使用用户ID选择网关
    pub async fn select_gateway_by_user(
        &self,
        user_id: &str,
    ) -> Result<Option<ServiceInfo>, Box<dyn std::error::Error>> {
        self.select_gateway(Some(user_id)).await
    }

    /// 获取所有网关地址
    ///
    /// 返回所有网关的地址列表，格式：http://address:port
    pub async fn get_gateway_addresses(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let gateways = self.get_gateway_instances().await?;
        Ok(gateways
            .iter()
            .map(|g| format!("http://{}:{}", g.address, g.port))
            .collect())
    }

    /// 刷新缓存
    pub async fn refresh_cache(
        &self,
        service_type: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(service_type) = service_type {
            // 刷新指定服务类型的缓存
            let registry = self.registry.read().await;
            let services = registry.discover(service_type).await?;

            let mut cache = self.cache.write().await;
            let mut last_update = self.last_update.write().await;
            cache.insert(service_type.to_string(), services);
            last_update.insert(service_type.to_string(), std::time::Instant::now());
        } else {
            // 刷新所有缓存
            let registry = self.registry.read().await;
            let service_types = registry.list_service_types().await?;

            let mut cache = self.cache.write().await;
            let mut last_update = self.last_update.write().await;

            for service_type in service_types {
                if let Ok(services) = registry.discover(&service_type).await {
                    cache.insert(service_type.clone(), services);
                    last_update.insert(service_type, std::time::Instant::now());
                }
            }
        }

        Ok(())
    }

    /// 注册服务
    pub async fn register_service(
        &self,
        service: ServiceInfo,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut registry = self.registry.write().await;
        registry.register(service.clone()).await?;

        // 更新缓存
        let service_type = service.service_type.clone();
        let mut cache = self.cache.write().await;
        let mut last_update = self.last_update.write().await;
        let services = cache.entry(service_type.clone()).or_insert_with(Vec::new);
        if !services
            .iter()
            .any(|s| s.instance_id == service.instance_id)
        {
            services.push(service);
        }
        last_update.insert(service_type, std::time::Instant::now());

        Ok(())
    }

    /// 注销服务
    pub async fn unregister_service(
        &self,
        service_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut registry = self.registry.write().await;
        registry.unregister(service_id).await?;

        // 清除缓存
        let mut cache = self.cache.write().await;
        let mut last_update = self.last_update.write().await;
        cache.retain(|_, services| {
            services.retain(|s| s.instance_id != service_id);
            !services.is_empty()
        });
        last_update.clear();

        Ok(())
    }
}
