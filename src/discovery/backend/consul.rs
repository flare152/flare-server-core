//! Consul 服务发现后端

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use reqwest::Client as HttpClient;

use crate::discovery::{
    DiscoveryBackend, DiscoveryConfig, ServiceInstance,
};
use crate::kv::{KvBackend, KvEntry, KvError};
use base64::Engine;

/// Consul 服务发现后端
pub struct ConsulBackend {
    http_client: Arc<HttpClient>,
    consul_url: String,
    /// 默认命名空间（用于 unregister 等操作）
    _default_namespace: String,
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
            _default_namespace: default_namespace,
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
impl KvBackend for ConsulBackend {
    /// 获取键值
    async fn get(&self, key: &str) -> Result<Option<KvEntry>, KvError> {
        let url = format!("{}/v1/kv/{}", self.consul_url, key.trim_start_matches('/'));
        
        match self.http_client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let body: Vec<serde_json::Value> = response.json().await
                        .map_err(|e| KvError::OperationFailed(format!("Failed to parse consul response: {}", e)))?;
                    
                    if let Some(first) = body.first() {
                        // 从响应中提取Value字段并进行base64解码
                        if let Some(value_base64) = first.get("Value").and_then(|v| v.as_str()) {
                            // base64解码
                            let value_bytes = base64::engine::general_purpose::STANDARD.decode(value_base64)
                                .map_err(|e| KvError::OperationFailed(format!("Failed to decode base64 value from consul: {}", e)))?;
                            
                            let entry = KvEntry {
                                key: first.get("Key").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                value: value_bytes,
                                version: first.get("Version").and_then(|v| v.as_u64()).unwrap_or(0),
                                create_revision: first.get("CreateIndex").and_then(|v| v.as_u64()).unwrap_or(0),
                                mod_revision: first.get("ModifyIndex").and_then(|v| v.as_u64()).unwrap_or(0),
                                lease: first.get("Lease").and_then(|v| v.as_i64()).unwrap_or(0),
                            };
                            
                            Ok(Some(entry))
                        } else {
                            Err(KvError::KeyNotFound(key.to_string()))
                        }
                    } else {
                        Ok(None)
                    }
                } else {
                    Err(KvError::OperationFailed(format!("Failed to get key from consul, status: {}", response.status())))
                }
            }
            Err(e) => {
                Err(KvError::OperationFailed(format!("Failed to get key from consul: {}", e)))
            }
        }
    }
    
    /// 获取多个键值
    async fn get_range(&self, key: &str, _range_end: &str) -> Result<Vec<KvEntry>, KvError> {
        // Consul 不直接支持范围查询，我们可以通过前缀查询来模拟
        let keys = self.prefix_keys(key).await?;
        let mut entries = Vec::new();
        for k in keys {
            let k_owned = k.clone();
            if let Some(entry) = self.get(&k_owned).await? {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
    
    /// 设置键值
    async fn put(&self, key: &str, value: &[u8]) -> Result<(), KvError> {
        let url = format!("{}/v1/kv/{}", self.consul_url, key.trim_start_matches('/'));
        let value_base64 = base64::engine::general_purpose::STANDARD.encode(value);
        
        match self.http_client.put(&url).body(value_base64).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(KvError::OperationFailed(format!("Failed to put key to consul, status: {}", response.status())))
                }
            }
            Err(e) => {
                Err(KvError::OperationFailed(format!("Failed to put key to consul: {}", e)))
            }
        }
    }
    
    /// 删除键
    async fn delete(&self, key: &str) -> Result<bool, KvError> {
        let url = format!("{}/v1/kv/{}", self.consul_url, key.trim_start_matches('/'));
        
        match self.http_client.delete(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    Ok(true)
                } else {
                    Err(KvError::OperationFailed(format!("Failed to delete key from consul, status: {}", response.status())))
                }
            }
            Err(e) => {
                Err(KvError::OperationFailed(format!("Failed to delete key from consul: {}", e)))
            }
        }
    }
    
    /// 删除范围内的键
    async fn delete_range(&self, key: &str, _range_end: &str) -> Result<u64, KvError> {
        // Consul 不直接支持范围删除，我们可以通过前缀查询来模拟
        let keys = self.prefix_keys(key).await?;
        let mut count = 0u64;
        
        for k in keys {
            if self.delete(&k).await? {
                count += 1;
            }
        }
        
        Ok(count)
    }
    
    /// 获取键的前缀列表
    async fn prefix_keys(&self, prefix: &str) -> Result<Vec<String>, KvError> {
        let url = format!("{}/v1/kv/{}?keys", self.consul_url, prefix.trim_start_matches('/'));
        
        match self.http_client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let keys: Vec<String> = response.json().await
                        .map_err(|e| KvError::OperationFailed(format!("Failed to parse consul response: {}", e)))?;
                    Ok(keys)
                } else {
                    Err(KvError::OperationFailed(format!("Failed to get prefix keys from consul, status: {}", response.status())))
                }
            }
            Err(e) => {
                Err(KvError::OperationFailed(format!("Failed to get prefix keys from consul: {}", e)))
            }
        }
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
        
        // 记录响应状态和原始内容（用于调试）
        let status = resp.status();
        let resp_text = resp.text().await
            .map_err(|e| format!("Failed to read response body: {}", e))?;
        
        tracing::debug!(
            service_type = %service_type,
            url = %url,
            status = %status,
            response_len = resp_text.len(),
            tag_filters = ?tags,
            "Consul service discovery response"
        );
        
        // 解析 JSON
        let services: Vec<serde_json::Value> = serde_json::from_str(&resp_text)
            .map_err(|e| {
                tracing::error!(
                    service_type = %service_type,
                    error = %e,
                    response_preview = %resp_text.chars().take(500).collect::<String>(),
                    "Failed to parse Consul response as JSON"
                );
                format!("error decoding response body: {} (response preview: {})", e, resp_text.chars().take(200).collect::<String>())
            })?;
        
        let total_services_count = services.len();
        tracing::debug!(
            service_type = %service_type,
            services_count = total_services_count,
            "Parsed Consul services"
        );
        
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
            
            // 记录实例的标签信息（用于调试）
            let instance_tags: Vec<String> = instance.tags.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            tracing::debug!(
                service_type = %service_type,
                instance_id = %instance.instance_id,
                address = %instance.address,
                tags = ?instance_tags,
                "Found service instance from Consul"
            );
            
            // 过滤命名空间
            if let Some(ns) = namespace {
                if !instance.matches_namespace(Some(ns)) {
                    tracing::debug!(
                        instance_id = %instance.instance_id,
                        namespace = %ns,
                        "Instance filtered out by namespace"
                    );
                    continue;
                }
            }
            
            // 过滤版本
            if let Some(ver) = version {
                if !instance.matches_version(Some(ver)) {
                    tracing::debug!(
                        instance_id = %instance.instance_id,
                        version = %ver,
                        "Instance filtered out by version"
                    );
                    continue;
                }
            }
            
            // 过滤标签
            if let Some(tag_filters) = tags {
                let tag_filter_str: Vec<String> = tag_filters.iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                if !instance.matches_tags(tag_filters) {
                    tracing::debug!(
                        instance_id = %instance.instance_id,
                        instance_tags = ?instance_tags,
                        required_tags = ?tag_filter_str,
                        "Instance filtered out by tag filters"
                    );
                    continue;
                } else {
                    tracing::debug!(
                        instance_id = %instance.instance_id,
                        instance_tags = ?instance_tags,
                        required_tags = ?tag_filter_str,
                        "Instance passed tag filters"
                    );
                }
            }
            
            instances.push(instance);
        }
        
        tracing::info!(
            service_type = %service_type,
            total_found = total_services_count,
            filtered_count = instances.len(),
            tag_filters = ?tags,
            "Consul service discovery completed"
        );
        
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
