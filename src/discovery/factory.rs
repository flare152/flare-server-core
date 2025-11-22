//! 服务发现工厂
//!
//! 提供快速构建方法，包含服务注册和发现，使用最优默认配置

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tracing::{info, warn, error};
use serde_json::json;

use crate::discovery::{
    DiscoveryBackend, BackendType, DiscoveryConfig, ServiceDiscover, ServiceDiscoverUpdater,
    ServiceInstance, HealthCheckConfig, LoadBalanceStrategy,
};
use crate::discovery::backend::etcd::EtcdBackend;
use crate::discovery::backend::consul::ConsulBackend;
use crate::discovery::backend::dns::DnsBackend;
use crate::discovery::backend::mesh::MeshBackend;

/// 服务发现工厂
pub struct DiscoveryFactory;

impl DiscoveryFactory {
    /// 从配置创建服务发现后端
    pub async fn create_backend(
        config: &DiscoveryConfig,
    ) -> Result<Arc<dyn DiscoveryBackend>, Box<dyn std::error::Error + Send + Sync>> {
        match config.backend {
            BackendType::Etcd => {
                let backend = EtcdBackend::new(config).await?;
                Ok(Arc::new(backend))
            }
            BackendType::Consul => {
                let backend = ConsulBackend::new(config).await?;
                Ok(Arc::new(backend))
            }
            BackendType::Dns => {
                let backend = DnsBackend::new(config).await?;
                Ok(Arc::new(backend))
            }
            BackendType::Mesh => {
                let backend = MeshBackend::new(config).await?;
                Ok(Arc::new(backend))
            }
        }
    }

    /// 从配置创建 ServiceDiscover（带自动刷新）
    pub async fn create_discover(
        config: DiscoveryConfig,
    ) -> Result<(ServiceDiscover, ServiceDiscoverUpdater), Box<dyn std::error::Error + Send + Sync>> {
        let backend = Self::create_backend(&config).await?;
        ServiceDiscover::from_backend(backend, config).await
    }

    /// 使用最优默认配置创建服务发现
    ///
    /// # 参数
    /// * `backend_type` - 后端类型
    /// * `backend_endpoints` - 后端地址列表（如 etcd endpoints）
    /// * `service_type` - 要发现的服务类型
    ///
    /// # 默认配置（可通过环境变量调整）
    /// - 心跳间隔：20 秒（可通过 `SERVICE_HEARTBEAT_INTERVAL` 调整）
    /// - TTL：45 秒（Consul，可通过 `CONSUL_TTL_SECONDS` 调整）或 60 秒（etcd，可通过 `ETCD_TTL_SECONDS` 调整）
    /// - 刷新间隔：30 秒
    /// - 健康检查：启用，间隔 10 秒，超时 5 秒
    /// - 负载均衡：P2C（Power of Two Choices）
    /// - 失败阈值：3 次
    /// - 成功阈值：2 次
    ///
    /// # 心跳配置建议
    /// - **快速感知**：`SERVICE_HEARTBEAT_INTERVAL=15 CONSUL_TTL_SECONDS=30` → 故障检测 < 30s
    /// - **平衡模式**（推荐）：`SERVICE_HEARTBEAT_INTERVAL=20 CONSUL_TTL_SECONDS=45` → 故障检测 < 45s
    /// - **宽松模式**：`SERVICE_HEARTBEAT_INTERVAL=30 CONSUL_TTL_SECONDS=60` → 故障检测 < 60s
    pub async fn create_with_defaults(
        backend_type: BackendType,
        backend_endpoints: Vec<String>,
        service_type: String,
    ) -> Result<(ServiceDiscover, ServiceDiscoverUpdater), Box<dyn std::error::Error + Send + Sync>> {
        let mut backend_config = HashMap::new();
        
        match backend_type {
            BackendType::Etcd => {
                backend_config.insert("endpoints".to_string(), json!(backend_endpoints));
                backend_config.insert("service_type".to_string(), json!(service_type));
                // etcd TTL 可通过环境变量调整，默认 60 秒（心跳间隔 20 秒的 3 倍）
                let etcd_ttl = std::env::var("ETCD_TTL_SECONDS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(60); // 默认 60 秒
                backend_config.insert("ttl".to_string(), json!(etcd_ttl));
            }
            BackendType::Consul => {
                backend_config.insert("url".to_string(), json!(backend_endpoints.first().unwrap_or(&"http://localhost:8500".to_string())));
                backend_config.insert("service_type".to_string(), json!(service_type));
            }
            BackendType::Dns => {
                backend_config.insert("domain".to_string(), json!("local"));
                backend_config.insert("service_type".to_string(), json!(service_type));
            }
            BackendType::Mesh => {
                backend_config.insert("xds_server".to_string(), json!(backend_endpoints.first().unwrap_or(&"http://localhost:8080".to_string())));
                backend_config.insert("service_type".to_string(), json!(service_type));
            }
        }

        let config = DiscoveryConfig {
            backend: backend_type,
            backend_config,
            namespace: None,
            version: None,
            tag_filters: vec![],
            load_balance: LoadBalanceStrategy::ConsistentHash, // 默认一致性哈希，适合大多数场景
            health_check: Some(HealthCheckConfig {
                interval: 10,  // 健康检查间隔：10 秒（心跳间隔的 1/3）
                timeout: 5,    // 超时：5 秒
                failure_threshold: 3,  // 连续失败 3 次后标记为不健康
                success_threshold: 2,  // 连续成功 2 次后标记为健康
                path: Some("/health".to_string()),
            }),
            refresh_interval: Some(30), // 刷新间隔：30 秒（与心跳间隔一致）
        };

        Self::create_discover(config).await
    }

    /// 快速构建：服务注册 + 发现（推荐用于生产环境）
    ///
    /// 同时提供服务注册和发现功能，自动处理心跳续期和生命周期管理
    ///
    /// # 参数
    /// * `backend_type` - 后端类型
    /// * `backend_endpoints` - 后端地址列表
    /// * `service_type` - 服务类型
    /// * `instance` - 要注册的服务实例
    ///
    /// # 返回
    /// * `ServiceRegistry` - 服务注册器（自动处理心跳和注销）
    /// * `ServiceDiscover` - 服务发现器
    /// * `ServiceDiscoverUpdater` - 服务发现更新器
    ///
    /// # 默认配置（可通过环境变量调整）
    /// - 心跳间隔：20 秒（平衡检测速度和开销，可通过 `SERVICE_HEARTBEAT_INTERVAL` 环境变量调整）
    /// - TTL：45 秒（心跳间隔的 2.25 倍，可通过 `CONSUL_TTL_SECONDS` 环境变量调整）
    /// - 刷新间隔：30 秒（与服务发现同步）
    /// - 健康检查：启用，间隔 10 秒
    ///
    /// # 心跳配置建议
    /// - **快速感知**（关键服务）：心跳 15s，TTL 30s → 故障检测 < 30s
    /// - **平衡模式**（推荐）：心跳 20s，TTL 45s → 故障检测 < 45s
    /// - **宽松模式**（非关键服务）：心跳 30s，TTL 60s → 故障检测 < 60s
    pub async fn register_and_discover(
        backend_type: BackendType,
        backend_endpoints: Vec<String>,
        service_type: String,
        instance: ServiceInstance,
    ) -> Result<(ServiceRegistry, ServiceDiscover, ServiceDiscoverUpdater), Box<dyn std::error::Error + Send + Sync>> {
        // 创建后端
        let mut backend_config = HashMap::new();
        match backend_type {
            BackendType::Etcd => {
                backend_config.insert("endpoints".to_string(), json!(backend_endpoints));
                backend_config.insert("service_type".to_string(), json!(service_type.clone()));
                backend_config.insert("ttl".to_string(), json!(90)); // TTL = 心跳间隔 * 3
            }
            BackendType::Consul => {
                backend_config.insert("url".to_string(), json!(backend_endpoints.first().unwrap_or(&"http://localhost:8500".to_string())));
                backend_config.insert("service_type".to_string(), json!(service_type.clone()));
            }
            _ => {
                return Err("DNS 和 Mesh 后端不支持服务注册".into());
            }
        }

        let config = DiscoveryConfig {
            backend: backend_type,
            backend_config,
            namespace: None,
            version: None,
            tag_filters: vec![],
            load_balance: LoadBalanceStrategy::ConsistentHash,
            health_check: Some(HealthCheckConfig {
                interval: 10,
                timeout: 5,
                failure_threshold: 3,
                success_threshold: 2,
                path: Some("/health".to_string()),
            }),
            refresh_interval: Some(30),
        };

        let backend = Self::create_backend(&config).await?;
        
        // 注册服务实例
        backend.register(instance.clone()).await?;
        info!(
            service_type = %service_type,
            instance_id = %instance.instance_id,
            address = %instance.address,
            "✅ Service registered"
        );

        // 创建服务发现器
        let (discover, updater) = ServiceDiscover::from_backend(backend.clone(), config.clone()).await?;

        // 创建服务注册器（自动处理心跳和注销）
        // 心跳机制通过 DiscoveryBackend::heartbeat() 统一处理，各后端自行实现
        // 心跳间隔可通过环境变量调整，默认 20 秒（平衡检测速度和开销）
        let heartbeat_interval = std::env::var("SERVICE_HEARTBEAT_INTERVAL")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(20); // 默认 20 秒（平衡模式）
        
        let registry = ServiceRegistry::new(
            backend,
            instance,
            heartbeat_interval,
        );

        Ok((registry, discover, updater))
    }
}

/// 服务注册器
///
/// 自动处理服务注册、心跳续期和注销
/// 从架构角度考虑：
/// - 心跳间隔：30 秒（最优：平衡网络开销和故障检测速度）
/// - TTL：心跳间隔的 2-3 倍（60-90 秒），确保网络抖动时不会误判
/// - 优雅关闭：自动注销服务实例
/// - 统一接口：通过 `DiscoveryBackend::heartbeat()` 方法统一处理心跳，各后端自行实现
pub struct ServiceRegistry {
    backend: Arc<dyn DiscoveryBackend>,
    instance: ServiceInstance,
    #[allow(dead_code)] // 保留用于未来扩展（如动态调整心跳间隔）
    heartbeat_interval: u64,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl ServiceRegistry {
    /// 创建新的服务注册器
    ///
    /// # 参数
    /// * `backend` - 服务发现后端
    /// * `instance` - 服务实例
    /// * `heartbeat_interval` - 心跳间隔（秒），默认 20 秒（可通过 `SERVICE_HEARTBEAT_INTERVAL` 环境变量调整）
    ///
    /// # 说明
    /// 心跳机制通过 `DiscoveryBackend::heartbeat()` 方法统一处理，
    /// 各后端自行实现最适合的心跳方式：
    /// - **etcd**: 重新注册服务以续期 lease TTL（默认实现）
    /// - **Consul**: 调用 TTL 更新 API（ConsulBackend 重写实现）
    /// - **DNS/Mesh**: 使用默认实现或重写
    pub fn new(
        backend: Arc<dyn DiscoveryBackend>,
        instance: ServiceInstance,
        heartbeat_interval: u64,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let instance_clone = instance.clone();
        let backend_clone = backend.clone();
        let interval_secs = heartbeat_interval;

        // 启动心跳任务
        // 使用统一的 heartbeat() 方法，各后端自行实现最适合的心跳方式
        // 心跳间隔设置为 TTL 的 1/2，确保在 TTL 过期前至少发送 2 次心跳
        tokio::spawn(async move {
            let mut interval_timer = interval(Duration::from_secs(interval_secs));
            let mut shutdown_rx = shutdown_rx;
            
            // 立即发送一次心跳，确保服务注册后立即更新 TTL 状态
            match backend_clone.heartbeat(&instance_clone).await {
                Ok(_) => {
                    tracing::debug!(
                        instance_id = %instance_clone.instance_id,
                        "💓 Initial heartbeat sent"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        instance_id = %instance_clone.instance_id,
                        error = %e,
                        "⚠️ Failed to send initial heartbeat"
                    );
                }
            }
            
            loop {
                tokio::select! {
                    _ = interval_timer.tick() => {
                        // 发送心跳：通过统一的 heartbeat() 方法
                        // 各后端自行实现最适合的心跳方式
                        match backend_clone.heartbeat(&instance_clone).await {
                            Ok(_) => {
                                tracing::debug!(
                                    instance_id = %instance_clone.instance_id,
                                    "💓 Heartbeat sent"
                                );
                            }
                            Err(e) => {
                                error!(
                                    instance_id = %instance_clone.instance_id,
                                    error = %e,
                                    "❌ Failed to send heartbeat"
                                );
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!(
                            instance_id = %instance_clone.instance_id,
                            "🛑 Heartbeat task stopped"
                        );
                        break;
                    }
                }
            }
        });

        Self {
            backend,
            instance,
            heartbeat_interval,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    /// 获取服务实例
    pub fn instance(&self) -> &ServiceInstance {
        &self.instance
    }

    /// 手动发送心跳（通常不需要，自动心跳会处理）
    pub async fn heartbeat(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.backend.register(self.instance.clone()).await
    }

    /// 更新服务实例信息（如健康状态、权重等）
    pub async fn update_instance(&mut self, instance: ServiceInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.instance = instance.clone();
        self.backend.register(instance).await
    }
}

impl ServiceRegistry {
    /// 优雅关闭：停止心跳并注销服务
    ///
    /// 应该在服务关闭前显式调用此方法，而不是依赖 Drop
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            // 停止心跳任务
            let _ = shutdown_tx.send(());
            
            // 等待一小段时间，确保心跳任务已停止
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            
            // 注销服务实例
            let instance_id = self.instance.instance_id.clone();
            match self.backend.unregister(&instance_id).await {
                Ok(_) => {
                    info!(
                        instance_id = %instance_id,
                        "✅ Service unregistered"
                    );
                    Ok(())
                }
                Err(e) => {
                    warn!(
                        instance_id = %instance_id,
                        error = %e,
                        "⚠️ Failed to unregister service"
                    );
                    Err(e)
                }
            }
        } else {
            // 已经关闭过了
            Ok(())
        }
    }
}

impl Drop for ServiceRegistry {
    fn drop(&mut self) {
        // 如果 shutdown_tx 还存在，说明没有显式调用 shutdown
        // 尝试在 Drop 中注销（但可能失败，因为 runtime 可能正在关闭）
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            // 停止心跳任务
            let _ = shutdown_tx.try_send(());
            
            // 尝试在 Drop 中注销（使用 try_current 检查 runtime 是否可用）
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let backend = self.backend.clone();
                let instance_id = self.instance.instance_id.clone();
                // 使用 spawn 但不等待，因为 Drop 是同步的
                handle.spawn(async move {
                    // 短暂延迟，确保心跳任务已停止
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    match backend.unregister(&instance_id).await {
                        Ok(_) => {
                            tracing::info!(
                                instance_id = %instance_id,
                                "✅ Service unregistered (from Drop)"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                instance_id = %instance_id,
                                error = %e,
                                "⚠️ Failed to unregister service (from Drop)"
                            );
                        }
                    }
                });
            } else {
                // Runtime 不可用，无法注销
                tracing::warn!(
                    instance_id = %self.instance.instance_id,
                    "⚠️ Cannot unregister service: tokio runtime not available"
                );
            }
        }
    }
}

