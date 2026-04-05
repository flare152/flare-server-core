//! 指标收集模块

use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::Request;

/// 指标数据
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub requests_total: u64,
    pub requests_success: u64,
    pub requests_failed: u64,
    pub request_duration_ms: Vec<u64>,
}

/// 指标收集器
#[derive(Clone)]
pub struct MetricsCollector {
    metrics: Arc<RwLock<Metrics>>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(Metrics::default())),
        }
    }

    pub async fn record_request(&self, success: bool, duration: std::time::Duration) {
        let mut metrics = self.metrics.write().await;
        metrics.requests_total += 1;

        if success {
            metrics.requests_success += 1;
        } else {
            metrics.requests_failed += 1;
        }

        metrics
            .request_duration_ms
            .push(duration.as_millis() as u64);

        // 只保留最近 1000 个请求的耗时
        if metrics.request_duration_ms.len() > 1000 {
            metrics.request_duration_ms.remove(0);
        }
    }

    pub async fn get_metrics(&self) -> Metrics {
        self.metrics.read().await.clone()
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// 指标拦截器
pub struct MetricsInterceptor {
    collector: MetricsCollector,
}

impl MetricsInterceptor {
    pub fn new(collector: MetricsCollector) -> Self {
        Self { collector }
    }

    pub fn intercept<T>(&self, req: Request<T>) -> Request<T> {
        // 指标记录在服务实现中完成
        req
    }

    pub fn collector(&self) -> MetricsCollector {
        self.collector.clone()
    }
}
