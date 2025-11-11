//! 负载均衡模块
//!
//! 提供多种负载均衡策略，用于从多个服务实例中选择一个

use crate::types::ServiceInfo;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

/// 负载均衡策略
#[derive(Debug, Clone, Copy)]
pub enum LoadBalanceStrategy {
    /// 轮询（Round Robin）
    RoundRobin,
    /// 随机（Random）
    Random,
    /// 一致性哈希（Consistent Hash）
    ConsistentHash,
    /// 最少连接（Least Connections）- 需要额外的连接数统计
    LeastConnections,
}

/// 负载均衡器
pub struct LoadBalancer {
    strategy: LoadBalanceStrategy,
    round_robin_index: AtomicUsize,
    connection_counts: std::sync::Arc<tokio::sync::RwLock<HashMap<String, usize>>>,
}

impl LoadBalancer {
    /// 创建新的负载均衡器
    pub fn new(strategy: LoadBalanceStrategy) -> Self {
        Self {
            strategy,
            round_robin_index: AtomicUsize::new(0),
            connection_counts: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// 选择服务实例
    pub async fn select<'a>(
        &self,
        services: &'a [ServiceInfo],
        key: Option<&str>,
    ) -> Option<&'a ServiceInfo> {
        if services.is_empty() {
            return None;
        }

        match self.strategy {
            LoadBalanceStrategy::RoundRobin => self.select_round_robin(services),
            LoadBalanceStrategy::Random => self.select_random(services),
            LoadBalanceStrategy::ConsistentHash => {
                self.select_consistent_hash(services, key.unwrap_or(""))
            }
            LoadBalanceStrategy::LeastConnections => self.select_least_connections(services).await,
        }
    }

    /// 轮询选择
    fn select_round_robin<'a>(&self, services: &'a [ServiceInfo]) -> Option<&'a ServiceInfo> {
        let index = self.round_robin_index.fetch_add(1, Ordering::Relaxed);
        services.get(index % services.len())
    }

    /// 随机选择
    fn select_random<'a>(&self, services: &'a [ServiceInfo]) -> Option<&'a ServiceInfo> {
        use std::collections::hash_map::DefaultHasher;
        use std::time::{SystemTime, UNIX_EPOCH};

        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        let hash = hasher.finish();

        services.get((hash as usize) % services.len())
    }

    /// 一致性哈希选择
    fn select_consistent_hash<'a>(
        &self,
        services: &'a [ServiceInfo],
        key: &str,
    ) -> Option<&'a ServiceInfo> {
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let hash = hasher.finish();

        services.get((hash as usize) % services.len())
    }

    /// 最少连接选择
    async fn select_least_connections<'a>(
        &self,
        services: &'a [ServiceInfo],
    ) -> Option<&'a ServiceInfo> {
        let counts = self.connection_counts.read().await;

        services
            .iter()
            .min_by_key(|service| counts.get(&service.instance_id).copied().unwrap_or(0))
    }

    /// 增加连接数
    pub async fn increment_connections(&self, service_id: &str) {
        let mut counts = self.connection_counts.write().await;
        *counts.entry(service_id.to_string()).or_insert(0) += 1;
    }

    /// 减少连接数
    pub async fn decrement_connections(&self, service_id: &str) {
        let mut counts = self.connection_counts.write().await;
        if let Some(count) = counts.get_mut(service_id) {
            if *count > 0 {
                *count -= 1;
            }
        }
    }
}

impl Default for LoadBalancer {
    fn default() -> Self {
        Self::new(LoadBalanceStrategy::RoundRobin)
    }
}

/// 服务选择器
///
/// 封装负载均衡逻辑，提供便捷的服务选择接口
pub struct ServiceSelector {
    balancer: LoadBalancer,
}

impl ServiceSelector {
    /// 创建新的服务选择器
    pub fn new(strategy: LoadBalanceStrategy) -> Self {
        Self {
            balancer: LoadBalancer::new(strategy),
        }
    }

    /// 选择服务实例
    pub async fn select_service(
        &self,
        services: &[ServiceInfo],
        key: Option<&str>,
    ) -> Option<ServiceInfo> {
        self.balancer.select(services, key).await.map(|s| s.clone())
    }

    /// 选择服务地址（格式：http://address:port）
    pub async fn select_address(
        &self,
        services: &[ServiceInfo],
        key: Option<&str>,
    ) -> Option<String> {
        self.balancer
            .select(services, key)
            .await
            .map(|s| format!("http://{}:{}", s.address, s.port))
    }
}

impl Default for ServiceSelector {
    fn default() -> Self {
        Self::new(LoadBalanceStrategy::RoundRobin)
    }
}
