//! Consul 服务注册发现实现

use super::trait_def::ServiceRegistryTrait;
use crate::types::ServiceInfo;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

/// Consul 服务注册发现
pub struct ConsulRegistry {
    client: reqwest::Client,
    base_url: String,
    namespace: String,
    ttl: u64,
    service_id: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct ConsulService {
    ID: String,
    Name: String,
    Tags: Vec<String>,
    Address: String,
    Port: u16,
    Check: ConsulCheck,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct ConsulCheck {
    HTTP: Option<String>,
    Interval: String,
    Timeout: String,
    DeregisterCriticalServiceAfter: String,
}

impl ConsulRegistry {
    pub async fn new(
        consul_endpoints: Vec<String>,
        namespace: String,
        ttl: u64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let base_url = consul_endpoints
            .first()
            .ok_or("No Consul endpoint provided")?
            .clone();

        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
            namespace,
            ttl,
            service_id: None,
        })
    }
}

#[async_trait]
impl ServiceRegistryTrait for ConsulRegistry {
    async fn register(&mut self, service: ServiceInfo) -> Result<(), Box<dyn std::error::Error>> {
        let service_id = format!("{}-{}", service.service_type, service.instance_id);
        self.service_id = Some(service_id.clone());

        let consul_service = ConsulService {
            ID: service_id.clone(),
            Name: format!("{}/{}", self.namespace, service.service_type),
            Tags: vec![],
            Address: service.address.clone(),
            Port: service.port,
            Check: ConsulCheck {
                HTTP: Some(format!(
                    "http://{}:{}/health",
                    service.address, service.port
                )),
                Interval: format!("{}s", self.ttl / 3),
                Timeout: "5s".to_string(),
                DeregisterCriticalServiceAfter: format!("{}s", self.ttl),
            },
        };

        let url = format!("{}/v1/agent/service/register", self.base_url);
        self.client
            .put(&url)
            .json(&consul_service)
            .send()
            .await
            .map_err(|e| format!("Failed to register service with Consul: {}", e))?;

        info!(
            "Service registered with Consul: {} at {}:{}",
            service.service_type, service.address, service.port
        );

        Ok(())
    }

    async fn unregister(&mut self, service_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!(
            "{}/v1/agent/service/deregister/{}",
            self.base_url, service_id
        );
        self.client
            .put(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to unregister service from Consul: {}", e))?;

        info!("Service unregistered from Consul: {}", service_id);
        Ok(())
    }

    async fn discover(
        &self,
        service_type: &str,
    ) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
        let service_name = format!("{}/{}", self.namespace, service_type);
        let url = format!(
            "{}/v1/health/service/{}?passing=true",
            self.base_url, service_name
        );

        let response: Vec<ConsulHealthResponse> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to discover services from Consul: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse Consul response: {}", e))?;

        let mut services = Vec::new();
        for item in response {
            let service = item.Service;
            services.push(ServiceInfo {
                service_type: service_type.to_string(),
                service_id: service.Service.clone(),
                instance_id: service.ID.clone(),
                address: service.Address,
                port: service.Port,
                metadata: HashMap::new(),
            });
        }

        Ok(services)
    }

    async fn get_service(
        &self,
        service_id: &str,
    ) -> Result<Option<ServiceInfo>, Box<dyn std::error::Error>> {
        let url = format!("{}/v1/agent/service/{}", self.base_url, service_id);

        let service: Option<ConsulService> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to get service from Consul: {}", e))?
            .json()
            .await
            .ok();

        if let Some(service) = service {
            Ok(Some(ServiceInfo {
                service_type: service.Name.clone(),
                service_id: service.ID.clone(),
                instance_id: service.ID,
                address: service.Address,
                port: service.Port,
                metadata: HashMap::new(),
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_service_types(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let url = format!("{}/v1/catalog/services", self.base_url);
        let services: HashMap<String, Vec<String>> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to list service types from Consul: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse Consul response: {}", e))?;

        Ok(services.keys().cloned().collect())
    }

    async fn list_all_services(&self) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
        let url = format!("{}/v1/catalog/services", self.base_url);
        let service_names: HashMap<String, Vec<String>> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to list services from Consul: {}", e))?
            .json()
            .await
            .map_err(|e| format!("Failed to parse Consul response: {}", e))?;

        let mut all_services = Vec::new();
        for service_name in service_names.keys() {
            if let Ok(services) = self.discover(service_name).await {
                all_services.extend(services);
            }
        }

        Ok(all_services)
    }

    async fn health_check(&self) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/v1/status/leader", self.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Consul health check failed: {}", e))?;
        Ok(())
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize)]
struct ConsulHealthResponse {
    Service: ConsulServiceResponse,
}

#[allow(non_snake_case)]
#[derive(Deserialize)]
struct ConsulServiceResponse {
    ID: String,
    #[serde(rename = "Service")]
    Service: String,
    Address: String,
    Port: u16,
}
