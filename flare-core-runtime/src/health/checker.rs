//! 健康检查器实现
//!
//! 定期执行健康检查并在失败时触发回调

use super::HealthCheck;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// 健康检查结果
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    /// 检查名称
    pub name: String,
    /// 是否成功
    pub healthy: bool,
    /// 错误信息（如果失败）
    pub error: Option<String>,
}

/// 健康检查器
///
/// 定期执行健康检查并在失败时触发回调
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::health::{HealthChecker, HealthCheck};
/// use std::sync::Arc;
///
/// let mut checker = HealthChecker::new();
/// checker.add_check(Arc::new(MyHealthCheck));
///
/// // 执行健康检查
/// let results = checker.check_all().await;
/// ```
pub struct HealthChecker {
    checks: Vec<Arc<dyn HealthCheck>>,
    failure_counts: Arc<RwLock<std::collections::HashMap<String, u32>>>,
    failure_threshold: u32,
    on_failure: Option<Arc<dyn Fn(&str) + Send + Sync>>,
}

impl HealthChecker {
    /// 创建新的健康检查器
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
            failure_counts: Arc::new(RwLock::new(std::collections::HashMap::new())),
            failure_threshold: 3,
            on_failure: None,
        }
    }

    /// 设置失败阈值
    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }

    /// 设置失败回调
    pub fn with_on_failure(mut self, callback: Arc<dyn Fn(&str) + Send + Sync>) -> Self {
        self.on_failure = Some(callback);
        self
    }

    /// 添加健康检查
    ///
    /// # 参数
    ///
    /// * `check` - 健康检查实现
    pub fn add_check(&mut self, check: Arc<dyn HealthCheck>) {
        let name = check.name().to_string();
        self.checks.push(check);
        info!(check_name = %name, "Health check added");
    }

    /// 执行所有健康检查
    ///
    /// # 返回
    ///
    /// 返回所有检查的结果列表
    pub async fn check_all(&self) -> Vec<HealthCheckResult> {
        let mut results = Vec::new();

        for check in &self.checks {
            let result = self.check_one(check.as_ref()).await;
            results.push(result);
        }

        results
    }

    /// 执行单个健康检查
    async fn check_one(&self, check: &dyn HealthCheck) -> HealthCheckResult {
        let name = check.name().to_string();

        match check.check().await {
            Ok(_) => {
                // 重置失败计数
                let mut counts = self.failure_counts.write().await;
                counts.insert(name.clone(), 0);

                HealthCheckResult {
                    name: name.clone(),
                    healthy: true,
                    error: None,
                }
            }
            Err(e) => {
                // 增加失败计数
                let mut counts = self.failure_counts.write().await;
                let count = counts.entry(name.clone()).or_insert(0);
                *count += 1;

                error!(
                    check_name = %name,
                    failure_count = *count,
                    threshold = self.failure_threshold,
                    error = %e,
                    "Health check failed"
                );

                // 检查是否超过阈值
                if *count >= self.failure_threshold {
                    warn!(
                        check_name = %name,
                        "Health check failure threshold exceeded"
                    );

                    // 触发回调
                    if let Some(callback) = &self.on_failure {
                        callback(&name);
                    }
                }

                HealthCheckResult {
                    name: name.clone(),
                    healthy: false,
                    error: Some(e.to_string()),
                }
            }
        }
    }

    /// 检查是否所有检查都健康
    pub async fn is_healthy(&self) -> bool {
        let results = self.check_all().await;
        results.iter().all(|r| r.healthy)
    }

    /// 获取检查数量
    pub fn check_count(&self) -> usize {
        self.checks.len()
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HealthError;
    use std::future::Future;
    use std::pin::Pin;

    struct TestHealthCheck {
        name: String,
        healthy: bool,
    }

    impl HealthCheck for TestHealthCheck {
        fn check(&self) -> Pin<Box<dyn Future<Output = Result<(), HealthError>> + Send + '_>> {
            let healthy = self.healthy;
            let name = self.name.clone();
            Box::pin(async move {
                if healthy {
                    Ok(())
                } else {
                    Err(HealthError::CheckFailed {
                        name,
                        reason: "Test failure".to_string(),
                    })
                }
            })
        }

        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_health_checker_add() {
        let mut checker = HealthChecker::new();
        checker.add_check(Arc::new(TestHealthCheck {
            name: "test-check".to_string(),
            healthy: true,
        }));

        assert_eq!(checker.check_count(), 1);
    }

    #[tokio::test]
    async fn test_health_checker_healthy() {
        let mut checker = HealthChecker::new();
        checker.add_check(Arc::new(TestHealthCheck {
            name: "test-check".to_string(),
            healthy: true,
        }));

        let is_healthy = checker.is_healthy().await;
        assert!(is_healthy);
    }

    #[tokio::test]
    async fn test_health_checker_unhealthy() {
        let mut checker = HealthChecker::new();
        checker.add_check(Arc::new(TestHealthCheck {
            name: "test-check".to_string(),
            healthy: false,
        }));

        let is_healthy = checker.is_healthy().await;
        assert!(!is_healthy);
    }
}
