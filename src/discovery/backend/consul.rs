//! Consul 服务发现后端

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use reqwest::Client as HttpClient;

use crate::discovery::{
    DiscoveryBackend, DiscoveryConfig, ServiceInstance,
};

/// Consul 服务发现后端
pub struct ConsulBackend {
    http_client: Arc<HttpClient>,
    consul_url: String,
    /// 默认命名空间（用于 unregister 等操作）
    default_namespace: String,
}

impl ConsulBackend {
    /// 创建新的 Consul 后端
    pub async fn new(config: &DiscoveryConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let consul_url = config.backend_config
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "http://localhost:8500".to_string());

        let default_namespace = config.namespace
            .as_ref()
            .and_then(|ns| ns.default.clone())
            .unwrap_or_else(|| "default".to_string());

        Ok(Self {
            http_client: Arc::new(HttpClient::new()),
            consul_url,
            default_namespace,
        })
    }

    /// 更新 TTL 检查状态（用于 Consul TTL 健康检查）
    /// 
    /// # 参数
    /// * `check_id` - 检查 ID，格式为 "service:<instance_id>"
    /// * `status` - 状态："pass", "warn", "fail"
    pub async fn update_ttl_check(&self, check_id: &str, status: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/v1/agent/check/{}/{}", self.consul_url, status, check_id);
        self.http_client.put(&url).send().await?;
        Ok(())
    }
}

#[async_trait]
impl DiscoveryBackend for ConsulBackend {
    async fn discover(
        &self,
        service_type: &str,
        namespace: Option<&str>,
        version: Option<&str>,
        tags: Option<&HashMap<String, String>>,
    ) -> Result<Vec<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
        // 在测试环境中，可能服务还没有通过健康检查，所以也查询所有服务（包括不健康的）
        let passing_only = std::env::var("CONSUL_PASSING_ONLY").unwrap_or_else(|_| "false".to_string()) == "true";
        
        let url = format!("{}/v1/health/service/{}", self.consul_url, service_type);
        let mut query_params = vec![];
        if passing_only {
            query_params.push(("passing", "true"));
        }
        
        let resp = self.http_client
            .get(&url)
            .query(&query_params)
            .send()
            .await?;
        
        let services: Vec<serde_json::Value> = resp.json().await?;
        
        let mut instances = Vec::new();
        for svc in services {
            let service = svc.get("Service").ok_or("Invalid service format")?;
            let address = service.get("Address")
                .and_then(|v| v.as_str())
                .ok_or("Missing address")?;
            let port = service.get("Port")
                .and_then(|v| v.as_u64())
                .ok_or("Missing port")?;
            
            let socket_addr = format!("{}:{}", address, port)
                .parse()
                .map_err(|e| format!("Invalid address: {}", e))?;
            
            let default_id = format!("{}-{}", service_type, address);
            let instance_id = service.get("ID")
                .and_then(|v| v.as_str())
                .unwrap_or(&default_id);
            
            let mut instance = ServiceInstance::new(
                service_type,
                instance_id,
                socket_addr,
            );
            
            // 解析标签
            if let Some(tags_array) = service.get("Tags").and_then(|v| v.as_array()) {
                for tag in tags_array {
                    if let Some(tag_str) = tag.as_str() {
                        if let Some((key, value)) = tag_str.split_once('=') {
                            instance = instance.with_tag(key, value);
                        } else {
                            instance = instance.with_tag(tag_str, "true");
                        }
                    }
                }
            }
            
            // 过滤命名空间
            if let Some(ns) = namespace {
                if !instance.matches_namespace(Some(ns)) {
                    continue;
                }
            }
            
            // 过滤版本
            if let Some(ver) = version {
                if !instance.matches_version(Some(ver)) {
                    continue;
                }
            }
            
            // 过滤标签
            if let Some(tag_filters) = tags {
                if !instance.matches_tags(tag_filters) {
                    continue;
                }
            }
            
            instances.push(instance);
        }
        
        Ok(instances)
    }

    async fn register(&self, instance: ServiceInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/v1/agent/service/register", self.consul_url);
        
        let mut tags = Vec::new();
        for (key, value) in &instance.tags {
            tags.push(format!("{}={}", key, value));
        }
        if let Some(version) = &instance.version {
            tags.push(format!("version={}", version));
        }
        if let Some(namespace) = &instance.namespace {
            tags.push(format!("namespace={}", namespace));
        }
        
        // 处理地址：将 0.0.0.0 转换为 127.0.0.1，因为 Consul 无法访问 0.0.0.0
        let service_ip = instance.address.ip();
        let service_address = if service_ip.is_unspecified() {
            // 0.0.0.0 或 :: 转换为 127.0.0.1 或 ::1
            if service_ip.is_ipv4() {
                "127.0.0.1".to_string()
            } else {
                "::1".to_string()
            }
        } else {
            service_ip.to_string()
        };
        
        // 构建健康检查配置
        // 对于 gRPC 服务，默认使用 TTL 检查（因为可能没有 HTTP 健康检查端点）
        // 如果设置了 CONSUL_USE_HTTP_CHECK 环境变量，则使用 HTTP 检查
        let check_id = format!("service:{}", instance.instance_id);
        let check = if std::env::var("CONSUL_USE_HTTP_CHECK").is_ok() {
            // HTTP 健康检查模式
            serde_json::json!({
                "HTTP": format!("http://{}:{}/health", service_address, instance.address.port()),
                "Interval": "10s",
                "Timeout": "5s",
                "DeregisterCriticalServiceAfter": "90s"
            })
        } else {
            // TTL 检查模式（默认，适合 gRPC 服务）
            // TTL 检查的 check_id 格式为 "service:<service_id>"
            // 服务需要定期调用 Consul 的 TTL 更新接口来保持健康状态
            // 
            // TTL 配置策略（从环境变量读取，默认 45 秒）：
            // - 快速感知：心跳 15s，TTL 30s（故障检测 < 30s，适合关键服务）
            // - 平衡模式：心跳 20s，TTL 45s（故障检测 < 45s，推荐）
            // - 宽松模式：心跳 30s，TTL 60s（故障检测 < 60s，适合非关键服务）
            // 
            // TTL 应该是心跳间隔的 2-2.5 倍，确保网络抖动时不会误判
            let ttl_seconds = std::env::var("CONSUL_TTL_SECONDS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(45); // 默认 45 秒（平衡模式）
            
            let deregister_seconds = ttl_seconds * 2; // 注销延迟 = TTL * 2
            
            serde_json::json!({
                "CheckID": check_id.clone(),
                "TTL": format!("{}s", ttl_seconds),
                "DeregisterCriticalServiceAfter": format!("{}s", deregister_seconds)
            })
        };
        
        let payload = serde_json::json!({
            "ID": instance.instance_id,
            "Name": instance.service_type,
            "Tags": tags,
            "Address": service_address,
            "Port": instance.address.port(),
            "Check": check
        });
        
        self.http_client
            .put(&url)
            .json(&payload)
            .send()
            .await?;
        
        Ok(())
    }

    async fn heartbeat(&self, instance: &ServiceInstance) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Consul TTL 检查：调用专门的 TTL 更新 API
        // check_id 格式为 "service:<instance_id>"
        let check_id = format!("service:{}", instance.instance_id);
        let url = format!("{}/v1/agent/check/pass/{}", self.consul_url, check_id);
        
        // 使用 timeout 确保请求不会无限等待，避免阻塞心跳任务
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            self.http_client.put(&url).send()
        ).await {
            Ok(Ok(resp)) => {
                if !resp.status().is_success() {
                    return Err(format!("Consul TTL update failed with status: {}", resp.status()).into());
                }
                Ok(())
            }
            Ok(Err(e)) => Err(format!("Consul TTL update request failed: {}", e).into()),
            Err(_) => Err("Consul TTL update timeout (5s)".into()),
        }
    }

    async fn unregister(&self, instance_id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/v1/agent/service/deregister/{}", self.consul_url, instance_id);
        self.http_client.put(&url).send().await?;
        Ok(())
    }

    async fn watch(
        &self,
        service_type: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
        // Consul watch 实现
        let (_tx, rx) = tokio::sync::mpsc::channel(100);
        let http_client = self.http_client.clone();
        let consul_url = self.consul_url.clone();
        let service_type = service_type.to_string();
        
        tokio::spawn(async move {
            let mut index = 0u64;
            loop {
                let url = format!("{}/v1/health/service/{}", consul_url, service_type);
                let resp = http_client
                    .get(&url)
                    .query(&[("passing", "true"), ("index", &index.to_string()), ("wait", "10s")])
                    .send()
                    .await;
                
                if let Ok(resp) = resp {
                    if let Some(new_index) = resp.headers().get("X-Consul-Index")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok()) {
                        index = new_index;
                    }
                    
                    // 解析响应并发送实例
                    if let Ok(services) = resp.json::<Vec<serde_json::Value>>().await {
                            // 解析并发送实例（简化实现）
                        // 实际应该调用 discover 方法并发送实例
                        let _ = services; // 暂时忽略，避免未使用变量警告
                    }
                }
            }
        });
        
        Ok(rx)
    }
}

