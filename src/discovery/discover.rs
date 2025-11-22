//! Tower Discover trait 实现
//!
//! 提供 Channel 缓存、P2C 负载均衡和简洁调用接口

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tower::discover::Change;
use tower::Service;
use futures::Stream;
use tonic::transport::Channel;

use crate::discovery::instance::ServiceInstance;
use crate::discovery::backend::DiscoveryBackend;
use crate::discovery::config::{DiscoveryConfig, TagFilter};

/// Channel 服务包装器
/// 
/// 实现 tower::Service trait，用于与 tower::balance 集成
/// 每个 Endpoint 对应一个缓存的 Channel
#[derive(Clone)]
pub struct ChannelService {
    channel: Channel,
}

impl ChannelService {
    /// 创建新的 Channel 服务
    pub fn new(channel: Channel) -> Self {
        Self { channel }
    }

    /// 获取底层的 Channel
    pub fn channel(&self) -> &Channel {
        &self.channel
    }
}

/// 实现 tower::Service trait
impl<Req> Service<Req> for ChannelService
where
    Req: Send + 'static,
{
    type Response = Channel;
    type Error = tower::BoxError;
    type Future = futures::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Req) -> Self::Future {
        // 返回缓存的 Channel
        futures::future::ready(Ok(self.channel.clone()))
    }
}

/// 实现 tower::load::Load trait
/// 
/// 用于负载均衡，返回服务的负载值
/// 这里返回一个固定值，表示所有服务负载相同
impl tower::load::Load for ChannelService {
    type Metric = u32;

    fn load(&self) -> Self::Metric {
        // 返回固定负载值，表示所有服务负载相同
        // 可以根据实际需求实现更复杂的负载计算逻辑
        0
    }
}

/// 实现 tower::discover::Discover 的服务发现器
///
/// 这是与 tower 生态系统集成的核心类型，可以用于：
/// - tower::balance::p2c::Balance 负载均衡
/// - tonic 客户端自动服务发现
/// - Channel 缓存：每个 Endpoint 对应一个 tonic::Channel
pub struct ServiceDiscover {
    // 接收 tower::discover::Change 事件的 channel
    rx: mpsc::Receiver<Change<String, ChannelService>>,
    // Channel 缓存：instance_id -> ChannelService
    // 注意：这个字段在 ServiceDiscoverUpdater 中使用，但编译器可能检测不到
    #[allow(dead_code)]
    channel_cache: Arc<RwLock<HashMap<String, ChannelService>>>,
    // 内部状态管理（用于后台刷新任务）
    #[allow(dead_code)] // 在 start_refresh_task 中使用
    backend: Arc<dyn DiscoveryBackend>,
    #[allow(dead_code)] // 在 start_refresh_task 中使用
    config: DiscoveryConfig,
    instances: Arc<RwLock<HashMap<String, ServiceInstance>>>,
}

/// ServiceDiscover 的更新器
/// 
/// 用于向 ServiceDiscover 发送服务变化事件
#[derive(Clone)]
pub struct ServiceDiscoverUpdater {
    tx: mpsc::Sender<Change<String, ChannelService>>,
    // Channel 缓存引用，用于创建和缓存 Channel
    channel_cache: Arc<RwLock<HashMap<String, ChannelService>>>,
}

impl ServiceDiscover {
    /// 创建新的服务发现器（内部方法）
    /// 
    /// 返回 (ServiceDiscover, ServiceDiscoverUpdater)
    /// - ServiceDiscover: 实现 tower::discover::Discover，用于与 tower::balance 集成
    /// - ServiceDiscoverUpdater: 用于发送服务变化事件
    /// 
    /// 注意：这个方法只创建 channel 和缓存，backend 和 config 需要在 from_backend 中设置
    /// 这里不创建实际的 backend，因为同步函数无法创建异步 backend
    /// 
    /// 这个方法不应该被直接调用，应该使用 `from_backend` 方法
    pub(crate) fn new_internal(
        buffer: usize,
        backend: Arc<dyn DiscoveryBackend>,
        config: DiscoveryConfig,
    ) -> (Self, ServiceDiscoverUpdater) {
        let (tx, rx) = mpsc::channel(buffer);
        let channel_cache = Arc::new(RwLock::new(HashMap::new()));
        
        (
            Self {
                rx,
                channel_cache: channel_cache.clone(),
            backend,
            config,
            instances: Arc::new(RwLock::new(HashMap::new())),
            },
            ServiceDiscoverUpdater {
                tx,
                channel_cache,
            },
        )
    }

    /// 从后端创建服务发现器（带自动刷新）
    pub async fn from_backend(
        backend: Arc<dyn DiscoveryBackend>,
        config: DiscoveryConfig,
    ) -> Result<(Self, ServiceDiscoverUpdater), Box<dyn std::error::Error + Send + Sync>> {
        let (discover, updater) = Self::new_internal(16, backend.clone(), config.clone());
        
        // 立即执行一次服务发现，确保至少有一些服务可用（如果有的话）
        let service_type = config.backend_config
            .get("service_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        
        // 立即执行一次服务发现
        if let Ok(new_instances) = backend.discover(&service_type, None, None, None).await {
            for new_inst in new_instances {
                let id = new_inst.instance_id.clone();
                // 创建 Channel 并缓存
                if let Ok(service) = updater.create_channel_service(&new_inst).await {
                    updater.insert(id.clone(), service).await;
                    // 更新内部实例映射
                    let mut current = discover.instances.write().await;
                    current.insert(id, new_inst);
                }
            }
        }
        
        // 启动后台刷新任务
        discover.start_refresh_task(backend, config, updater.clone()).await?;
        
        Ok((discover, updater))
    }

    /// 启动后台刷新任务
    async fn start_refresh_task(
        &self,
        backend: Arc<dyn DiscoveryBackend>,
        config: DiscoveryConfig,
        updater: ServiceDiscoverUpdater,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let service_type = config.backend_config
            .get("service_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        
        let interval = config.refresh_interval.unwrap_or(30);
        let instances_immediate = self.instances.clone();
        let instances_interval = self.instances.clone();
        let backend_clone = backend.clone();
        let updater_clone = updater.clone();
        let service_type_clone = service_type.clone();
        
        // 立即执行一次服务发现，不等待第一次 interval
        tokio::spawn(async move {
            // 立即执行一次服务发现
            if let Ok(new_instances) = backend_clone.discover(&service_type_clone, None, None, None).await {
                let mut current = instances_immediate.write().await;
                let new_map: HashMap<String, ServiceInstance> = new_instances
                    .into_iter()
                    .map(|inst| (inst.instance_id.clone(), inst))
                    .collect();
                
                // 检测变化并发送到 updater
                for (id, new_inst) in &new_map {
                    if !current.contains_key(id) {
                        // 新增：创建 Channel 并缓存
                        if let Ok(service) = updater_clone.create_channel_service(new_inst).await {
                            let _ = updater_clone.insert(id.clone(), service).await;
                        }
                        current.insert(id.clone(), new_inst.clone());
                    }
                }
            }
        });
        
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(tokio::time::Duration::from_secs(interval));
            // 跳过第一次 tick，因为我们已经立即执行了一次
            interval_timer.tick().await;
            
            loop {
                interval_timer.tick().await;
                
                match backend.discover(&service_type, None, None, None).await {
                    Ok(new_instances) => {
                        let mut current = instances_interval.write().await;
                        let new_map: HashMap<String, ServiceInstance> = new_instances
                            .into_iter()
                            .map(|inst| (inst.instance_id.clone(), inst))
                            .collect();
                        
                        // 检测变化并发送到 updater
                        for (id, new_inst) in &new_map {
                            match current.get(id) {
                                Some(old_inst) if old_inst != new_inst => {
                                    // 更新：先移除旧实例，再插入新实例
                                    let _ = updater.remove(id.clone()).await;
                                    // 创建新的 Channel 并缓存
                                    if let Ok(service) = updater.create_channel_service(new_inst).await {
                                        let _ = updater.insert(id.clone(), service).await;
                                    }
                                    current.insert(id.clone(), new_inst.clone());
                                }
                                None => {
                                    // 新增：创建 Channel 并缓存
                                    if let Ok(service) = updater.create_channel_service(new_inst).await {
                                        let _ = updater.insert(id.clone(), service).await;
                                    }
                                    current.insert(id.clone(), new_inst.clone());
                                }
                                _ => {}
                            }
                        }
                        
                        // 检测删除
                        let current_ids: Vec<String> = current.keys().cloned().collect();
                        for id in current_ids {
                            if !new_map.contains_key(&id) {
                                let _ = updater.remove(id.clone()).await;
                                current.remove(&id);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to refresh service instances");
                    }
                }
            }
        });
        
        Ok(())
    }


    /// 获取所有实例
    pub async fn get_instances(&self) -> Vec<ServiceInstance> {
        let instances = self.instances.read().await;
        instances.values().cloned().collect()
    }

    /// 根据标签过滤实例
    pub async fn filter_by_tags(&self, filters: &[TagFilter]) -> Vec<ServiceInstance> {
        let instances = self.instances.read().await;
        instances
            .values()
            .filter(|inst| {
                filters.iter().all(|filter| {
                    if let Some(value) = &filter.value {
                        inst.tags.get(&filter.key)
                            .map(|v| v == value)
                            .unwrap_or(false)
                    } else {
                        inst.tags.contains_key(&filter.key)
                    }
                })
            })
            .cloned()
            .collect()
    }
}

impl ServiceDiscoverUpdater {
    /// 创建 Channel 服务并缓存
    async fn create_channel_service(
        &self,
        instance: &ServiceInstance,
    ) -> Result<ChannelService, Box<dyn std::error::Error + Send + Sync>> {
        let uri = instance.to_grpc_uri();
        let instance_id = instance.instance_id.clone();
        
        // 检查缓存
        {
            let cache = self.channel_cache.read().await;
            if let Some(service) = cache.get(&instance_id) {
                return Ok(service.clone());
            }
        }
        
        // 创建新的 Channel
        let endpoint = tonic::transport::Endpoint::from_shared(uri)
            .map_err(|e| format!("Invalid URI: {}", e))?;
        let channel = endpoint.connect().await
            .map_err(|e| format!("Failed to connect: {}", e))?;
        
        let service = ChannelService::new(channel);
        
        // 缓存 Channel
        {
            let mut cache = self.channel_cache.write().await;
            cache.insert(instance_id, service.clone());
        }
        
        Ok(service)
    }

    /// 插入新服务实例（带 Channel 缓存）
    pub async fn insert(&self, key: impl Into<String>, service: ChannelService) {
        let key = key.into();
        // 更新缓存
        {
            let mut cache = self.channel_cache.write().await;
            cache.insert(key.clone(), service.clone());
        }
        let _ = self.tx.send(Change::Insert(key, service)).await;
    }

    /// 移除服务实例（清理 Channel 缓存）
    pub async fn remove(&self, key: impl Into<String>) {
        let key = key.into();
        // 清理缓存
        {
            let mut cache = self.channel_cache.write().await;
            cache.remove(&key);
        }
        let _ = self.tx.send(Change::Remove(key)).await;
                        }
                    }

/// 实现 futures::Stream trait
/// 
/// tower 0.5 的 Discover trait 通过 TryStream 自动实现
/// 我们实现 Stream<Item = Result<Change<...>, Infallible>>，这样会自动实现 TryStream
/// 从而自动实现 Discover trait
impl Stream for ServiceDiscover {
    type Item = Result<Change<String, ChannelService>, std::convert::Infallible>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(change)) => Poll::Ready(Some(Ok(change))),
            Poll::Ready(None) => Poll::Ready(None), // channel 关闭
            Poll::Pending => Poll::Pending,
        }
    }
}

// 注意：tower 0.5 的 Discover trait 通过 TryStream 自动实现
// 由于我们实现了 Stream<Item = Result<Change<String, ChannelService>, Infallible>>，
// 这会自动实现 TryStream，从而自动实现 Discover trait
// 所以我们不需要手动实现 Discover trait

/// 服务发现客户端
/// 
/// 提供简洁的调用接口，集成 Channel 缓存和 P2C 负载均衡
/// 
/// 注意：Balance 的第二个类型参数是请求类型，我们使用 () 作为占位符
/// 
/// 注意：ServiceClient 不支持 Clone，并发调用时请使用 Arc<Mutex<ServiceClient>>
pub struct ServiceClient {
    balancer: tower::balance::p2c::Balance<
        Pin<Box<ServiceDiscover>>,
        (),  // 请求类型是 ()，因为我们只需要获取 Channel
    >,
}

impl ServiceClient {
    /// 从 ServiceDiscover 创建客户端
    pub fn new(discover: ServiceDiscover) -> Self {
        // Balance 的第二个类型参数是请求类型，我们使用 () 作为占位符
        let balancer: tower::balance::p2c::Balance<Pin<Box<ServiceDiscover>>, ()> = 
            tower::balance::p2c::Balance::new(Box::pin(discover));
        Self { balancer }
    }

    /// 调用服务（简洁接口）
    /// 
    /// # 参数
    /// * `_req` - 请求（占位符，实际不使用）
    /// 
    /// # 返回
    /// 返回选中的 Channel，可用于创建 gRPC 客户端
    pub async fn call_service<Req>(
        &mut self,
        _req: Req,
    ) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
        use tower::Service;
        
        // Balance 实现了 Service trait
        // Balance 的 Request 类型是 ()，Response 是 Channel
        // call() 方法会自动处理 ready 检查，如果没有服务可用会等待
        // 调用时会自动选择一个 ChannelService，然后调用其 call(()) 方法返回 Channel
        let channel = self.balancer.call(()).await
            .map_err(|e| format!("Service call error: {}", e))?;
        
        Ok(channel)
    }

    /// 获取 Channel（不发送请求）
    /// 
    /// 用于创建 gRPC 客户端
    /// 
    /// # 注意
    /// 如果没有可用的服务实例，此方法会一直等待直到有服务可用。
    /// 建议在生产环境中使用超时包装此调用。
    pub async fn get_channel(&mut self) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
        use tower::Service;
        use futures::future::poll_fn;
        
        // Balance 实现了 Service trait
        // 我们需要先确保 Balance 已经准备好（即已经发现了至少一个服务）
        // 使用 poll_fn 配合 yield_now 来等待 ready 状态
        // 这样 Balance 内部的 Discover 才能处理 Stream 事件
        
        // 尝试最多 100 次（10 秒），每次等待 100ms
        for _attempt in 0..100 {
            // 让出执行权，允许 Balance 处理 Discover 的 Stream 事件
            tokio::task::yield_now().await;
            
            // 使用 poll_fn 来检查是否准备好
            // poll_fn 会在每次 poll 时调用 poll_ready，如果返回 Pending 会继续等待
            let ready_result = poll_fn(|cx| {
                self.balancer.poll_ready(cx)
            }).await;
            
            match ready_result {
                Ok(()) => {
                    // 服务已经准备好，可以调用
                    break;
                }
                Err(e) => {
                    return Err(format!("Service not ready: {}", e).into());
                }
            }
            
            // 如果还没准备好（这里实际上不会执行，因为 match 的所有分支都会返回或跳出）
            // 但保留这个 sleep 以防逻辑变化
            #[allow(unreachable_code)]
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        
        // 循环结束意味着服务已经准备好（通过 break 跳出）
        // 再次确认服务准备好（虽然理论上已经准备好了）
        poll_fn(|cx| {
            self.balancer.poll_ready(cx)
        }).await
            .map_err(|e| format!("Service not ready after waiting: {}", e))?;
        
        // Balance 的 Request 类型是 ()，Response 是 Channel
        // 调用时会自动选择一个 ChannelService，然后调用其 call(()) 方法返回 Channel
        let channel = self.balancer.call(()).await
            .map_err(|e| format!("Service call error: {}", e))?;
        
        Ok(channel)
    }
}

