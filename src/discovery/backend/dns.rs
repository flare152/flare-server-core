//! DNS/SRV 服务发现后端

use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;

use crate::discovery::{
    DiscoveryBackend, DiscoveryConfig, ServiceInstance,
};

/// DNS 服务发现后端
pub struct DnsBackend {
    config: DiscoveryConfig,
    namespace: String,
}

impl DnsBackend {
    /// 创建新的 DNS 后端
    pub async fn new(config: &DiscoveryConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let namespace = config.namespace
            .as_ref()
            .and_then(|ns| ns.default.clone())
            .unwrap_or_else(|| "default".to_string());

        Ok(Self {
            config: config.clone(),
            namespace,
        })
    }

    async fn resolve_srv(&self, _service_type: &str) -> Result<Vec<SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
        // DNS SRV 解析
        // 格式：_service._tcp.namespace.domain
        let _domain = self.config.backend_config
            .get("domain")
            .and_then(|v| v.as_str())
            .unwrap_or("local");
        
        // TODO: 实现实际的 DNS SRV 解析
        // let srv_name = format!("_{}._tcp.{}.{}", service_type, self.namespace, domain);
        
        // 使用 hickory-dns 或 trust-dns 进行 DNS 查询
        // 这里简化实现，返回配置的地址
        let addresses = self.config.backend_config
            .get("addresses")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .map(|v| {
                        v.as_str()
                            .and_then(|s| s.parse::<SocketAddr>().ok())
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .unwrap_or_default();
        
        Ok(addresses)
    }
}

#[async_trait]
impl DiscoveryBackend for DnsBackend {
    async fn discover(
        &self,
        service_type: &str,
        _namespace: Option<&str>,
        _version: Option<&str>,
        _tags: Option<&HashMap<String, String>>,
    ) -> Result<Vec<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
        let addresses = self.resolve_srv(service_type).await?;
        
        let mut instances = Vec::new();
        for (idx, addr) in addresses.iter().enumerate() {
            let instance = ServiceInstance::new(
                service_type,
                format!("{}-{}", service_type, idx),
                *addr,
            )
            .with_namespace(&self.namespace);
            
            instances.push(instance);
        }
        
        Ok(instances)
    }

    async fn register(&self, _instance: ServiceInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // DNS 后端通常不支持注册（只读）
        Err("DNS backend does not support registration".into())
    }

    async fn unregister(&self, _instance_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // DNS 后端通常不支持注销（只读）
        Err("DNS backend does not support unregistration".into())
    }

    async fn watch(
        &self,
        service_type: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
        // DNS 后端不支持 watch（需要轮询）
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        
        // 定期轮询 DNS
        let service_type = service_type.to_string();
        let backend = self.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if let Ok(instances) = backend.discover(&service_type, None, None, None).await {
                    for inst in instances {
                        let _ = tx.send(inst).await;
                    }
                }
            }
        });
        
        Ok(rx)
    }
}

impl Clone for DnsBackend {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            namespace: self.namespace.clone(),
        }
    }
}

