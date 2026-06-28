//! etcd 服务发现后端

use async_trait::async_trait;
use etcd_client::{Client, GetOptions, PutOptions, WatchOptions};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::discovery::{DiscoveryBackend, DiscoveryConfig, ServiceInstance};

/// etcd 服务发现后端
pub struct EtcdBackend {
    client: Arc<Mutex<Client>>,
    /// 默认命名空间（用于 unregister 等操作）
    default_namespace: String,
}

impl EtcdBackend {
    /// 创建新的 etcd 后端
    pub async fn new(
        config: &DiscoveryConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let endpoints = config
            .backend_config
            .get("endpoints")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| "etcd endpoints not configured".to_string())?;

        let client = Client::connect(&endpoints, None).await?;

        let default_namespace = config
            .namespace
            .as_ref()
            .and_then(|ns| ns.default.clone())
            .unwrap_or_else(|| "flare".to_string());

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            default_namespace,
        })
    }

    fn service_key(
        &self,
        service_type: &str,
        instance_id: Option<&str>,
        namespace: Option<&str>,
    ) -> String {
        let ns = namespace.unwrap_or(&self.default_namespace);
        if let Some(id) = instance_id {
            format!("{}/services/{}/{}", ns, service_type, id)
        } else {
            format!("{}/services/{}/", ns, service_type)
        }
    }
}

#[async_trait]
impl DiscoveryBackend for EtcdBackend {
    async fn discover(
        &self,
        service_type: &str,
        namespace: Option<&str>,
        version: Option<&str>,
        tags: Option<&HashMap<String, String>>,
    ) -> Result<Vec<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
        // 如果没有指定 namespace，需要搜索所有可能的 namespace
        // 简化实现：先尝试默认 namespace，如果指定了则只搜索指定的
        let mut key_prefixes = Vec::new();
        if let Some(ns) = namespace {
            key_prefixes.push(self.service_key(service_type, None, Some(ns)));
        } else {
            // 如果没有指定 namespace，搜索所有常见的 namespace
            // 注意：这是一个简化实现，实际应该支持配置多个 namespace 或搜索所有
            key_prefixes.push(self.service_key(service_type, None, Some("flare")));
            key_prefixes.push(self.service_key(service_type, None, Some("flare-test")));
            key_prefixes.push(self.service_key(service_type, None, Some("default")));
        }

        let mut client = self.client.lock().await;
        let mut instances = Vec::new();

        for key_prefix in key_prefixes {
            let opts = GetOptions::new().with_prefix();
            if let Ok(resp) = client.get(key_prefix.clone(), Some(opts)).await {
                for kv in resp.kvs() {
                    if let Ok(instance) = serde_json::from_slice::<ServiceInstance>(kv.value()) {
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
                }
            }
        }

        Ok(instances)
    }

    async fn register(
        &self,
        instance: ServiceInstance,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 使用 instance 中的 namespace，如果没有则使用默认值
        let namespace = instance.namespace.as_deref().unwrap_or("flare");
        let key = self.service_key(
            &instance.service_type,
            Some(&instance.instance_id),
            Some(namespace),
        );
        let value = serde_json::to_vec(&instance)?;

        let mut client = self.client.lock().await;
        // TTL 通过 lease 实现，这里简化处理
        let opts = PutOptions::new(); // 实际应该创建 lease 并设置 TTL
        client.put(key, value, Some(opts)).await?;

        Ok(())
    }

    async fn unregister(
        &self,
        instance_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 首先在默认 namespace 中查找并删除
        // 如果找不到，再搜索其他常见的 namespace
        let mut client = self.client.lock().await;

        // 先尝试默认 namespace
        let key_prefix = format!("{}/services/", self.default_namespace);
        let opts = GetOptions::new().with_prefix();

        if let Ok(resp) = client.get(key_prefix, Some(opts)).await {
            for kv in resp.kvs() {
                if let Ok(instance) = serde_json::from_slice::<ServiceInstance>(kv.value()) {
                    if instance.instance_id == instance_id {
                        let key = String::from_utf8_lossy(kv.key()).to_string();
                        client.delete(key, None).await?;
                        return Ok(());
                    }
                }
            }
        }

        // 如果默认 namespace 中没找到，搜索其他常见的 namespace
        let other_namespaces = vec!["flare-test", "default"];
        for namespace in other_namespaces {
            if namespace == self.default_namespace {
                continue; // 已经搜索过了
            }
            let key_prefix = format!("{}/services/", namespace);
            let opts = GetOptions::new().with_prefix();

            if let Ok(resp) = client.get(key_prefix, Some(opts)).await {
                for kv in resp.kvs() {
                    if let Ok(instance) = serde_json::from_slice::<ServiceInstance>(kv.value()) {
                        if instance.instance_id == instance_id {
                            let key = String::from_utf8_lossy(kv.key()).to_string();
                            client.delete(key, None).await?;
                            return Ok(());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn watch(
        &self,
        service_type: &str,
    ) -> Result<
        tokio::sync::mpsc::Receiver<ServiceInstance>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        // watch 时使用默认 namespace
        let key_prefix = self.service_key(service_type, None, Some(&self.default_namespace));
        let client = self.client.clone();
        let service_type = service_type.to_string();

        tokio::spawn(async move {
            // 退避重连：etcd 抖动/断流时不再 panic，自动重建 watch。
            const BACKOFF_MIN: std::time::Duration = std::time::Duration::from_millis(500);
            const BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(30);
            let mut backoff = BACKOFF_MIN;

            loop {
                // 消费端已关闭则退出，避免泄漏后台任务。
                if tx.is_closed() {
                    return;
                }

                // 仅在创建 watcher 期间持锁；随后释放，避免 watch 长期独占共享 client。
                let watch_result = {
                    let mut guard = client.lock().await;
                    let opts = WatchOptions::new().with_prefix();
                    guard.watch(key_prefix.clone(), Some(opts)).await
                };

                let (_watcher, mut stream) = match watch_result {
                    Ok(pair) => pair,
                    Err(error) => {
                        tracing::warn!(
                            %service_type,
                            %error,
                            retry_in = ?backoff,
                            "etcd watch 建立失败，退避后重试",
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(BACKOFF_MAX);
                        continue;
                    }
                };

                // 成功建立连接：重置退避。
                backoff = BACKOFF_MIN;

                loop {
                    match stream.message().await {
                        Ok(Some(resp)) => {
                            for event in resp.events() {
                                if let Some(kv) = event.kv() {
                                    if let Ok(instance) =
                                        serde_json::from_slice::<ServiceInstance>(kv.value())
                                    {
                                        if tx.send(instance).await.is_err() {
                                            return; // 消费端关闭
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            tracing::warn!(%service_type, "etcd watch 流结束，重建中");
                            break;
                        }
                        Err(error) => {
                            tracing::warn!(%service_type, %error, "etcd watch 流错误，重建中");
                            break;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}
