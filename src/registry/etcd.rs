//! etcd 服务注册发现实现

use super::trait_def::ServiceRegistryTrait;
use crate::types::ServiceInfo;
use async_trait::async_trait;
use etcd_client::{Client, PutOptions};
use std::time::Duration;
use tracing::{error, info};

/// etcd 服务注册发现
pub struct EtcdRegistry {
    client: Client,
    namespace: String,
    ttl: u64,
    lease_id: Option<i64>,
    keep_alive_handle: Option<tokio::task::JoinHandle<()>>,
}

impl EtcdRegistry {
    pub async fn new(
        etcd_endpoints: Vec<String>,
        namespace: String,
        ttl: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = Client::connect(etcd_endpoints, None)
            .await
            .map_err(|e| format!("Failed to connect to etcd: {}", e))?;

        Ok(Self {
            client,
            namespace,
            ttl,
            lease_id: None,
            keep_alive_handle: None,
        })
    }

    async fn start_keep_alive(&mut self, lease_id: i64) {
        let mut client = self.client.clone();
        let ttl = self.ttl as i64;

        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(ttl as u64 / 3)).await;

                match client.lease_keep_alive(lease_id).await {
                    Ok((_keeper, mut stream)) => {
                        // 使用 tokio_stream::StreamExt 的 next() 方法
                        use tokio_stream::StreamExt;
                        while let Some(resp) = stream.next().await {
                            match resp {
                                Ok(resp) => {
                                    info!(
                                        "Lease keep-alive successful for lease {}: {:?}",
                                        lease_id, resp
                                    );
                                }
                                Err(e) => {
                                    error!("Lease keep-alive stream error: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Lease keep-alive failed: {}", e);
                        break;
                    }
                }
            }
        });

        self.keep_alive_handle = Some(handle);
    }
}

#[async_trait]
impl ServiceRegistryTrait for EtcdRegistry {
    async fn register(&mut self, service: ServiceInfo) -> Result<(), Box<dyn std::error::Error>> {
        let lease = self
            .client
            .lease_grant(self.ttl as i64, None)
            .await
            .map_err(|e| format!("Failed to grant lease: {}", e))?;

        let lease_id = lease.id();
        self.lease_id = Some(lease_id);

        let key = format!(
            "/{}/{}/{}",
            self.namespace, service.service_type, service.instance_id
        );

        let value = serde_json::to_string(&service)
            .map_err(|e| format!("Failed to serialize service: {}", e))?;

        let mut opts = PutOptions::new();
        opts = opts.with_lease(lease_id);

        self.client
            .put(key, value, Some(opts))
            .await
            .map_err(|e| format!("Failed to register service: {}", e))?;

        info!(
            "Service registered: {} at {}:{}",
            service.service_type, service.address, service.port
        );

        self.start_keep_alive(lease_id).await;

        Ok(())
    }

    async fn unregister(&mut self, service_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        // 停止 keep-alive
        if let Some(handle) = self.keep_alive_handle.take() {
            handle.abort();
        }

        // 删除服务
        let key_prefix = format!("/{}/", self.namespace);
        let mut client = self.client.clone();
        let resp = client
            .get(key_prefix.clone(), None)
            .await
            .map_err(|e| format!("Failed to discover services: {}", e))?;

        for kv in resp.kvs() {
            if let Ok(service) = serde_json::from_slice::<ServiceInfo>(kv.value()) {
                if service.instance_id == service_id {
                    let key = String::from_utf8_lossy(kv.key());
                    client
                        .delete(key.to_string(), None)
                        .await
                        .map_err(|e| format!("Failed to unregister service: {}", e))?;
                    info!("Service unregistered: {}", service_id);
                    break;
                }
            }
        }

        Ok(())
    }

    async fn discover(
        &self,
        service_type: &str,
    ) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
        let key_prefix = format!("/{}/{}/", self.namespace, service_type);

        let mut client = self.client.clone();
        let resp = client
            .get(key_prefix.clone(), None)
            .await
            .map_err(|e| format!("Failed to discover services: {}", e))?;

        let mut services = Vec::new();

        for kv in resp.kvs() {
            if let Ok(service) = serde_json::from_slice::<ServiceInfo>(kv.value()) {
                services.push(service);
            }
        }

        Ok(services)
    }

    async fn get_service(
        &self,
        service_id: &str,
    ) -> Result<Option<ServiceInfo>, Box<dyn std::error::Error>> {
        let key_prefix = format!("/{}/", self.namespace);
        let mut client = self.client.clone();
        let resp = client
            .get(key_prefix.clone(), None)
            .await
            .map_err(|e| format!("Failed to discover services: {}", e))?;

        for kv in resp.kvs() {
            if let Ok(service) = serde_json::from_slice::<ServiceInfo>(kv.value()) {
                if service.instance_id == service_id {
                    return Ok(Some(service));
                }
            }
        }

        Ok(None)
    }

    async fn list_service_types(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let key_prefix = format!("/{}/", self.namespace);
        let mut client = self.client.clone();
        let resp = client
            .get(key_prefix.clone(), None)
            .await
            .map_err(|e| format!("Failed to list service types: {}", e))?;

        let mut service_types = std::collections::HashSet::new();
        for kv in resp.kvs() {
            if let Ok(service) = serde_json::from_slice::<ServiceInfo>(kv.value()) {
                service_types.insert(service.service_type);
            }
        }

        Ok(service_types.into_iter().collect())
    }

    async fn list_all_services(&self) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
        let key_prefix = format!("/{}/", self.namespace);
        let mut client = self.client.clone();
        let resp = client
            .get(key_prefix.clone(), None)
            .await
            .map_err(|e| format!("Failed to list all services: {}", e))?;

        let mut services = Vec::new();
        for kv in resp.kvs() {
            if let Ok(service) = serde_json::from_slice::<ServiceInfo>(kv.value()) {
                services.push(service);
            }
        }

        Ok(services)
    }

    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>> {
        // 简单的健康检查：尝试获取一个 key
        let mut client = self.client.clone();
        client
            .get("/health", None)
            .await
            .map_err(|e| format!("etcd health check failed: {}", e))?;
        Ok(())
    }
}

impl Drop for EtcdRegistry {
    fn drop(&mut self) {
        if let Some(handle) = self.keep_alive_handle.take() {
            handle.abort();
        }
    }
}
