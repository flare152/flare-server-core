//! 健康检查模块

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 健康状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Unknown,
    Serving,
    NotServing,
}

/// 健康检查服务
pub struct HealthService {
    statuses: Arc<RwLock<HashMap<String, HealthStatus>>>,
}

impl HealthService {
    pub fn new() -> Self {
        Self {
            statuses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn set_status(&self, service: impl Into<String>, status: HealthStatus) {
        let mut statuses = self.statuses.write().await;
        statuses.insert(service.into(), status);
    }

    pub async fn get_status(&self, service: &str) -> HealthStatus {
        let statuses = self.statuses.read().await;
        statuses
            .get(service)
            .copied()
            .unwrap_or(HealthStatus::Unknown)
    }

    pub async fn set_serving(&self) {
        let mut statuses = self.statuses.write().await;
        for status in statuses.values_mut() {
            *status = HealthStatus::Serving;
        }
    }

    pub async fn set_not_serving(&self) {
        let mut statuses = self.statuses.write().await;
        for status in statuses.values_mut() {
            *status = HealthStatus::NotServing;
        }
    }
}

impl Default for HealthService {
    fn default() -> Self {
        Self::new()
    }
}
