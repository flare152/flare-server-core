//! Tower Discover trait 实现
//!
//! 提供 Channel 缓存、P2C 负载均衡和简洁调用接口

use futures::{Stream, StreamExt};
use std::collections::{HashMap, HashSet, VecDeque};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tonic::transport::channel::Change as ChannelChange;
use tonic::transport::{Channel, Endpoint};
use tower::Service;
use tower::discover::Change;

use crate::discovery::backend::DiscoveryBackend;
use crate::discovery::balance::BalancedChannel;
use crate::discovery::config::{
    DiscoveryConfig, TagFilter, default_discovery_refresh_interval_secs,
};
use crate::discovery::instance::ServiceInstance;

/// 将配置中的 tag_filters 转为 backend.discover() 所需的 HashMap。
/// 仅包含 value 存在的过滤器（精确匹配）。
fn tag_filters_to_map(config: &DiscoveryConfig) -> Option<HashMap<String, String>> {
    let mut m = HashMap::new();
    for f in &config.tag_filters {
        if let Some(v) = &f.value {
            m.insert(f.key.clone(), v.clone());
        }
    }
    if m.is_empty() { None } else { Some(m) }
}

/// Channel 服务包装器
///
/// 实现 tower::Service trait，用于与 tower::balance 集成
/// 每个 Endpoint 对应一个缓存的 Channel
#[derive(Clone)]
pub struct ChannelService {
    channel: Channel,
    endpoint: Endpoint,
}

impl ChannelService {
    /// 创建新的 Channel 服务
    pub fn new(channel: Channel, endpoint: Endpoint) -> Self {
        Self { channel, endpoint }
    }

    /// 获取底层的 Channel
    pub fn channel(&self) -> &Channel {
        &self.channel
    }

    /// The endpoint is retained so a tonic balancing Channel can follow
    /// discovery insert/remove events instead of pinning one selected node.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum BalancerKey {
    Fallback,
    Instance(String),
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
    // Initial instances must not be sent to rx before the consumer exists:
    // doing so blocks construction once the fixed-size queue fills.
    initial_changes: VecDeque<Change<String, ChannelService>>,
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

/// HTTP/2 keepalive ping interval for balanced endpoints. A peer that stops
/// answering (for example a killed container whose address now black-holes
/// packets) is detected without waiting for TCP retransmission timeouts.
const BALANCED_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Bound every phase a balanced endpoint can hang in: connect, request
/// headers and keepalive acknowledgements.
fn tune_balanced_endpoint(endpoint: Endpoint, request_timeout: Duration) -> Endpoint {
    endpoint
        .timeout(request_timeout)
        .connect_timeout(request_timeout)
        .http2_keep_alive_interval(BALANCED_KEEPALIVE_INTERVAL)
        .keep_alive_timeout(request_timeout)
        .keep_alive_while_idle(true)
}

impl ServiceDiscover {
    /// Build one clonable [`BalancedChannel`] whose endpoint set follows
    /// discovery. A static fallback is used only while discovery has no
    /// registered nodes. Endpoints that fail at the connection level are
    /// isolated until a reconnect probe succeeds, so a dead replica stops
    /// receiving requests before discovery removes it. Requests wait while no
    /// endpoint is usable; callers must apply their own end-to-end deadline.
    /// Dropping all clones stops the forwarding task.
    pub fn into_balance_channel(
        mut self,
        fallback: Option<Endpoint>,
        request_timeout: Duration,
    ) -> BalancedChannel {
        let (channel, tx) = BalancedChannel::balance_channel(64, request_timeout);
        let fallback = fallback.map(|endpoint| tune_balanced_endpoint(endpoint, request_timeout));
        tokio::spawn(async move {
            let mut instances = HashSet::new();
            if let Some(endpoint) = fallback.as_ref()
                && tx
                    .send(ChannelChange::Insert(
                        BalancerKey::Fallback,
                        endpoint.clone(),
                    ))
                    .await
                    .is_err()
            {
                return;
            }
            loop {
                tokio::select! {
                    _ = tx.closed() => break,
                    event = self.next() => {
                        let Some(Ok(event)) = event else { break };
                        match event {
                            Change::Insert(id, service) => {
                                let first = instances.is_empty();
                                if tx.send(ChannelChange::Insert(BalancerKey::Instance(id.clone()), tune_balanced_endpoint(service.endpoint().clone(), request_timeout))).await.is_err() {
                                    break;
                                }
                                instances.insert(id);
                                if first && fallback.is_some() && tx.send(ChannelChange::Remove(BalancerKey::Fallback)).await.is_err() {
                                    break;
                                }
                            }
                            Change::Remove(id) => {
                                if instances.remove(&id) {
                                    if tx.send(ChannelChange::Remove(BalancerKey::Instance(id))).await.is_err() {
                                        break;
                                    }
                                    if instances.is_empty()
                                        && let Some(endpoint) = fallback.as_ref()
                                        && tx.send(ChannelChange::Insert(BalancerKey::Fallback, endpoint.clone())).await.is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
        channel
    }

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
                initial_changes: VecDeque::new(),
                channel_cache: channel_cache.clone(),
                backend,
                config,
                instances: Arc::new(RwLock::new(HashMap::new())),
            },
            ServiceDiscoverUpdater { tx, channel_cache },
        )
    }

    /// 从后端创建服务发现器（带自动刷新）
    pub async fn from_backend(
        backend: Arc<dyn DiscoveryBackend>,
        config: DiscoveryConfig,
    ) -> Result<(Self, ServiceDiscoverUpdater), Box<dyn std::error::Error + Send + Sync>> {
        // 立即执行一次服务发现，确保至少有一些服务可用（如果有的话）
        let service_type = config
            .backend_config
            .get("service_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tags_ref = tag_filters_to_map(&config);
        let namespace = config.namespace.as_ref().and_then(|n| n.default.as_deref());
        let version = config.version.as_ref().and_then(|v| v.default.as_deref());

        // Discover the initial snapshot before handing the consumer a stream.
        let initial = match backend
            .discover(&service_type, namespace, version, tags_ref.as_ref())
            .await
        {
            Ok(instances) => instances,
            Err(error) => {
                tracing::warn!(service_type = %service_type, %error, "initial service discovery failed");
                Vec::new()
            }
        };
        const MAX_INITIAL_INSTANCES: usize = 4096;
        if initial.len() > MAX_INITIAL_INSTANCES {
            return Err(format!(
                "service {service_type} has {} initial instances; limit is {MAX_INITIAL_INSTANCES}",
                initial.len()
            )
            .into());
        }
        let (mut discover, updater) = Self::new_internal(16, backend.clone(), config.clone());
        for new_inst in initial {
            let id = new_inst.instance_id.clone();
            match updater.create_channel_service(&new_inst).await {
                Ok(service) => {
                    discover
                        .initial_changes
                        .push_back(Change::Insert(id.clone(), service));
                    let mut current = discover.instances.write().await;
                    current.insert(id, new_inst);
                }
                Err(e) => {
                    tracing::warn!(
                        service_type = %service_type,
                        instance_id = %id,
                        address = %new_inst.address,
                        error = %e,
                        "service discover: failed to prepare gRPC channel for instance"
                    );
                }
            }
        }

        // 启动后台刷新任务
        discover
            .start_refresh_task(backend, config, updater.clone())
            .await?;

        Ok((discover, updater))
    }

    /// 启动后台刷新任务
    async fn start_refresh_task(
        &self,
        backend: Arc<dyn DiscoveryBackend>,
        config: DiscoveryConfig,
        updater: ServiceDiscoverUpdater,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let service_type = config
            .backend_config
            .get("service_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let interval = config
            .refresh_interval
            .unwrap_or_else(default_discovery_refresh_interval_secs);
        let instances_interval = self.instances.clone();
        let tags_ref = tag_filters_to_map(&config);
        let tags_interval = tags_ref.clone();
        let namespace = config
            .namespace
            .as_ref()
            .and_then(|n| n.default.as_deref())
            .map(String::from);
        let version = config
            .version
            .as_ref()
            .and_then(|v| v.default.as_deref())
            .map(String::from);
        let namespace_interval = namespace.clone();
        let version_interval = version.clone();

        tokio::spawn(async move {
            let mut interval_timer =
                tokio::time::interval(tokio::time::Duration::from_secs(interval));
            // from_backend 已做过首次 discover；跳过 interval 的第一次立即 tick。
            interval_timer.tick().await;

            loop {
                tokio::select! {
                    _ = updater.tx.closed() => break,
                    _ = interval_timer.tick() => {}
                }
                let ns = namespace_interval.as_deref();
                let ver = version_interval.as_deref();
                let result = tokio::select! {
                    _ = updater.tx.closed() => break,
                    result = backend.discover(&service_type, ns, ver, tags_interval.as_ref()) => result,
                };
                match result {
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
                                    if let Ok(service) =
                                        updater.create_channel_service(new_inst).await
                                    {
                                        let _ = updater.insert(id.clone(), service).await;
                                    }
                                    current.insert(id.clone(), new_inst.clone());
                                }
                                None => {
                                    // 新增：创建 Channel 并缓存
                                    if let Ok(service) =
                                        updater.create_channel_service(new_inst).await
                                    {
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
                        inst.tags
                            .get(&filter.key)
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

        // 延迟建连：避免插件尚未监听时 discover 失败导致 Balance 永远无可用实例。
        let endpoint = tonic::transport::Endpoint::from_shared(uri)
            .map_err(|e| format!("Invalid URI: {}", e))?;
        let channel = endpoint.connect_lazy();

        let service = ChannelService::new(channel, endpoint);

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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(change) = self.initial_changes.pop_front() {
            return Poll::Ready(Some(Ok(change)));
        }
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
/// 注意：ServiceClient 不支持 Clone，并发调用时请使用 `Arc<Mutex<ServiceClient>>`
pub struct ServiceClient {
    balancer: tower::balance::p2c::Balance<
        Pin<Box<ServiceDiscover>>,
        (), // 请求类型是 ()，因为我们只需要获取 Channel
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
        let channel = self
            .balancer
            .call(())
            .await
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
    pub async fn get_channel(
        &mut self,
    ) -> Result<Channel, Box<dyn std::error::Error + Send + Sync>> {
        use futures::future::poll_fn;
        use tower::Service;

        // Balance 实现了 Service trait。poll_ready 在没有实例时返回 Pending，
        // 并在 Discover 流送来 Insert 后被唤醒，所以这里一直等到至少有一个实例。
        // 本方法自身没有上限（见文档），调用方用 timeout 包住。
        poll_fn(|cx| self.balancer.poll_ready(cx))
            .await
            .map_err(|e| format!("Service not ready: {}", e))?;

        // Balance 的 Request 类型是 ()，Response 是 Channel
        // 调用时会自动选择一个 ChannelService，然后调用其 call(()) 方法返回 Channel
        let channel = self
            .balancer
            .call(())
            .await
            .map_err(|e| format!("Service call error: {}", e))?;

        Ok(channel)
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::discovery::config::{BackendType, LoadBalanceStrategy};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeBackend {
        initial: Vec<ServiceInstance>,
        calls: AtomicUsize,
        block_refresh: bool,
    }

    #[async_trait]
    impl DiscoveryBackend for FakeBackend {
        async fn discover(
            &self,
            _service_type: &str,
            _namespace: Option<&str>,
            _version: Option<&str>,
            _tags: Option<&HashMap<String, String>>,
        ) -> Result<Vec<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call > 0 && self.block_refresh {
                std::future::pending().await
            } else {
                Ok(self.initial.clone())
            }
        }

        async fn register(
            &self,
            _instance: ServiceInstance,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }

        async fn unregister(
            &self,
            _instance_id: &str,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }

        async fn watch(
            &self,
            _service_type: &str,
        ) -> Result<mpsc::Receiver<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(mpsc::channel(1).1)
        }
    }

    fn config(refresh_interval: u64) -> DiscoveryConfig {
        DiscoveryConfig {
            backend: BackendType::Dns,
            backend_config: HashMap::from([("service_type".into(), serde_json::json!("test"))]),
            namespace: None,
            version: None,
            tag_filters: vec![],
            load_balance: LoadBalanceStrategy::RoundRobin,
            health_check: None,
            refresh_interval: Some(refresh_interval),
        }
    }

    #[tokio::test]
    async fn initial_snapshot_over_queue_capacity_returns_and_emits_every_node() {
        for count in [17, 65] {
            let initial = (0..count)
                .map(|id| {
                    ServiceInstance::new(
                        "test",
                        format!("node-{id}"),
                        format!("127.0.0.1:{}", 20000 + id).parse().unwrap(),
                    )
                })
                .collect();
            let backend = Arc::new(FakeBackend {
                initial,
                calls: AtomicUsize::new(0),
                block_refresh: false,
            });
            let (mut discover, updater) = tokio::time::timeout(
                Duration::from_secs(1),
                ServiceDiscover::from_backend(backend, config(3600)),
            )
            .await
            .expect("initial discovery must not block on its own queue")
            .unwrap();
            assert_eq!(discover.get_instances().await.len(), count);
            for _ in 0..count {
                assert!(matches!(
                    discover.next().await,
                    Some(Ok(Change::Insert(_, _)))
                ));
            }
            drop(discover);
            drop(updater);
        }
    }

    #[tokio::test]
    async fn dropping_discover_cancels_pending_refresh_and_releases_backend() {
        let backend = Arc::new(FakeBackend {
            initial: vec![],
            calls: AtomicUsize::new(0),
            block_refresh: true,
        });
        let weak = Arc::downgrade(&backend);
        let (discover, updater) = ServiceDiscover::from_backend(backend.clone(), config(1))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while backend.calls.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("refresh should start");
        drop(backend);
        drop(discover);
        drop(updater);
        tokio::time::timeout(Duration::from_secs(1), async {
            while weak.strong_count() != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("refresh must exit when its receiver is dropped");
    }

    /// get_channel 在没有实例时一直挂起（不报错、不自带上限），
    /// 实例经 Discover 流到达后立刻返回。
    #[tokio::test]
    async fn get_channel_waits_for_first_instance_then_returns() {
        let backend = Arc::new(FakeBackend {
            initial: vec![],
            calls: AtomicUsize::new(0),
            block_refresh: false,
        });
        let (discover, updater) = ServiceDiscover::from_backend(backend, config(3600))
            .await
            .unwrap();
        let mut client = ServiceClient::new(discover);
        let pending = tokio::spawn(async move { client.get_channel().await.map(|_| ()) });

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!pending.is_finished(), "no instance yet: must keep waiting");

        let endpoint = Endpoint::from_static("http://127.0.0.1:20001");
        updater
            .insert(
                "node-1",
                ChannelService::new(endpoint.connect_lazy(), endpoint),
            )
            .await;
        tokio::time::timeout(Duration::from_secs(2), pending)
            .await
            .expect("must return once an instance arrives")
            .unwrap()
            .expect("ready balancer yields a channel");
    }
}

/// In-process failover test: two real tonic servers behind a controllable
/// discovery backend. One replica is stopped without a discovery Remove,
/// which is what callers see between a crash and the registry noticing it.
#[cfg(test)]
mod balance_failover_tests {
    use super::*;
    use crate::discovery::config::{BackendType, LoadBalanceStrategy};
    use async_trait::async_trait;
    use std::convert::Infallible;
    use std::fmt;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tonic::body::Body;
    use tonic::server::NamedService;
    use tonic::transport::Server;
    use tower::ServiceExt;

    const CALLS: usize = 1000;
    const CONCURRENCY: usize = 16;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

    /// Unary handler answering trailers-only OK and counting hits.
    #[derive(Clone)]
    struct Ping {
        hits: Arc<AtomicUsize>,
    }

    impl NamedService for Ping {
        const NAME: &'static str = "test.Ping";
    }

    impl Service<http::Request<Body>> for Ping {
        type Response = http::Response<Body>;
        type Error = Infallible;
        type Future = futures::future::Ready<Result<Self::Response, Infallible>>;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _request: http::Request<Body>) -> Self::Future {
            self.hits.fetch_add(1, Ordering::Relaxed);
            let response = http::Response::builder()
                .header("content-type", "application/grpc")
                .header("grpc-status", "0")
                .body(Body::empty())
                .expect("static response");
            futures::future::ready(Ok(response))
        }
    }

    /// Each replica owns an OS thread and runtime. Stopping it drops the
    /// runtime, which closes the listener and every accepted connection the
    /// way a crashed process would.
    struct Replica {
        addr: SocketAddr,
        hits: Arc<AtomicUsize>,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Replica {
        fn start(addr: SocketAddr) -> Self {
            let listener = std::net::TcpListener::bind(addr).expect("bind replica");
            listener
                .set_nonblocking(true)
                .expect("nonblocking listener");
            let addr = listener.local_addr().expect("replica address");
            let hits = Arc::new(AtomicUsize::new(0));
            let service = Ping { hits: hits.clone() };
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let thread = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("replica runtime");
                runtime.block_on(async move {
                    let listener =
                        tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                    let incoming = tonic::transport::server::TcpIncoming::from(listener);
                    tokio::select! {
                        _ = Server::builder().add_service(service).serve_with_incoming(incoming) => {}
                        _ = stopped => {}
                    }
                });
                drop(runtime);
            });
            Self {
                addr,
                hits,
                stop: Some(stop),
                thread: Some(thread),
            }
        }

        fn any_port() -> Self {
            Self::start("127.0.0.1:0".parse().expect("loopback address"))
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::Relaxed)
        }

        fn kill(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            if let Some(thread) = self.thread.take() {
                thread.join().expect("replica thread");
            }
        }
    }

    impl Drop for Replica {
        fn drop(&mut self) {
            self.kill();
        }
    }

    /// Discovery backend that only answers the initial snapshot; the test
    /// drives Remove through the updater.
    struct StaticBackend {
        instances: Vec<ServiceInstance>,
    }

    #[async_trait]
    impl DiscoveryBackend for StaticBackend {
        async fn discover(
            &self,
            _service_type: &str,
            _namespace: Option<&str>,
            _version: Option<&str>,
            _tags: Option<&HashMap<String, String>>,
        ) -> Result<Vec<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.instances.clone())
        }

        async fn register(
            &self,
            _instance: ServiceInstance,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }

        async fn unregister(
            &self,
            _instance_id: &str,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            Ok(())
        }

        async fn watch(
            &self,
            _service_type: &str,
        ) -> Result<mpsc::Receiver<ServiceInstance>, Box<dyn std::error::Error + Send + Sync>>
        {
            Ok(mpsc::channel(1).1)
        }
    }

    fn config() -> DiscoveryConfig {
        DiscoveryConfig {
            backend: BackendType::Dns,
            backend_config: HashMap::from([("service_type".into(), serde_json::json!("test"))]),
            namespace: None,
            version: None,
            tag_filters: vec![],
            load_balance: LoadBalanceStrategy::RoundRobin,
            health_check: None,
            // No periodic refresh: only the test decides when Remove happens.
            refresh_interval: Some(3600),
        }
    }

    async fn discovered_pair(
        a: &Replica,
        b: &Replica,
    ) -> (BalancedChannel, ServiceDiscoverUpdater) {
        let backend = Arc::new(StaticBackend {
            instances: vec![
                ServiceInstance::new("test", "a", a.addr),
                ServiceInstance::new("test", "b", b.addr),
            ],
        });
        let (discover, updater) = ServiceDiscover::from_backend(backend, config())
            .await
            .expect("discover");
        (
            discover.into_balance_channel(None, REQUEST_TIMEOUT),
            updater,
        )
    }

    fn ping_request() -> http::Request<Body> {
        http::Request::builder()
            .method("POST")
            .uri("/test.Ping/Call")
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(Body::empty())
            .expect("static request")
    }

    struct Stats {
        calls: usize,
        errors: usize,
        p50: Duration,
        p99: Duration,
    }

    impl Stats {
        fn error_rate(&self) -> f64 {
            self.errors as f64 / self.calls as f64
        }
    }

    impl fmt::Debug for Stats {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "calls={} errors={} error_rate={:.1}% p50={:?} p99={:?}",
                self.calls,
                self.errors,
                self.error_rate() * 100.0,
                self.p50,
                self.p99
            )
        }
    }

    async fn run_calls<S>(channel: &S, calls: usize) -> Stats
    where
        S: Service<http::Request<Body>, Response = http::Response<Body>> + Clone + Send + 'static,
        S::Future: Send,
    {
        use futures::StreamExt;
        let mut results: Vec<(Duration, bool)> = futures::stream::iter(0..calls)
            .map(|_| {
                let channel = channel.clone();
                async move {
                    let started = Instant::now();
                    let ok = matches!(
                        tokio::time::timeout(REQUEST_TIMEOUT, channel.oneshot(ping_request())).await,
                        Ok(Ok(response)) if response.headers().get("grpc-status").is_some_and(|status| status == "0")
                    );
                    (started.elapsed(), ok)
                }
            })
            .buffer_unordered(CONCURRENCY)
            .collect()
            .await;
        results.sort_by_key(|(latency, _)| *latency);
        let percentile =
            |p: f64| results[((results.len() as f64 * p).ceil() as usize).max(1) - 1].0;
        Stats {
            calls,
            errors: results.iter().filter(|(_, ok)| !ok).count(),
            p50: percentile(0.50),
            p99: percentile(0.99),
        }
    }

    /// Give the forwarding task and the balancer a moment to apply a change.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stopped_replica_is_isolated_before_discovery_removes_it() {
        let a = Replica::any_port();
        let mut b = Replica::any_port();
        let (channel, updater) = discovered_pair(&a, &b).await;

        let healthy = run_calls(&channel, CALLS).await;
        assert_eq!(healthy.errors, 0, "both replicas up: {healthy:?}");
        assert!(
            a.hits() > 0 && b.hits() > 0,
            "both replicas must take traffic"
        );

        b.kill();
        let stopped = run_calls(&channel, CALLS).await;
        updater.remove("b").await;
        settle().await;
        let removed = run_calls(&channel, CALLS).await;

        println!(
            "BalancedChannel stop-one: healthy [{healthy:?}] stopped-without-Remove [{stopped:?}] after-Remove [{removed:?}]"
        );
        assert!(
            stopped.error_rate() <= 0.05,
            "a stopped replica must be isolated after its first connection failures: {stopped:?}"
        );
        assert!(stopped.p99 < REQUEST_TIMEOUT, "{stopped:?}");
        assert_eq!(removed.errors, 0, "after Remove: {removed:?}");
    }

    /// Characterizes tonic's own balance channel, which this module replaces:
    /// lazy endpoints keep reporting ready after connection failures, so P2C
    /// keeps routing to the stopped replica until discovery removes it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tonic_balance_channel_keeps_routing_to_stopped_replica() {
        let a = Replica::any_port();
        let mut b = Replica::any_port();
        let (channel, tx) = Channel::balance_channel::<&'static str>(64);
        for (key, replica) in [("a", &a), ("b", &b)] {
            let endpoint = Endpoint::from_shared(format!("http://{}", replica.addr))
                .expect("endpoint")
                .timeout(REQUEST_TIMEOUT);
            tx.send(ChannelChange::Insert(key, endpoint))
                .await
                .expect("insert");
        }
        settle().await;

        let healthy = run_calls(&channel, CALLS).await;
        assert_eq!(healthy.errors, 0, "both replicas up: {healthy:?}");

        b.kill();
        let stopped = run_calls(&channel, CALLS).await;
        tx.send(ChannelChange::Remove("b")).await.expect("remove");
        settle().await;
        let removed = run_calls(&channel, CALLS).await;

        println!(
            "tonic balance_channel stop-one: healthy [{healthy:?}] stopped-without-Remove [{stopped:?}] after-Remove [{removed:?}]"
        );
        assert!(
            stopped.error_rate() >= 0.10,
            "tonic now isolates failed endpoints itself; revisit BalancedChannel: {stopped:?}"
        );
        assert_eq!(removed.errors, 0, "after Remove: {removed:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_every_balanced_channel_clone_releases_discovery() {
        let a = Replica::any_port();
        let backend = Arc::new(StaticBackend {
            instances: vec![ServiceInstance::new("test", "a", a.addr)],
        });
        let weak = Arc::downgrade(&backend);
        let (discover, updater) = ServiceDiscover::from_backend(backend, config())
            .await
            .expect("discover");
        let channel = discover.into_balance_channel(None, REQUEST_TIMEOUT);
        let clone = channel.clone();
        assert_eq!(run_calls(&clone, 10).await.errors, 0);
        drop(updater);
        drop(channel);
        drop(clone);
        tokio::time::timeout(Duration::from_secs(2), async {
            while weak.strong_count() != 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("forwarding and refresh tasks must exit once the channel is gone");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn isolated_replica_rejoins_after_restart_without_rediscovery() {
        let a = Replica::any_port();
        let mut b = Replica::any_port();
        let (channel, _updater) = discovered_pair(&a, &b).await;
        assert_eq!(run_calls(&channel, 200).await.errors, 0);

        let b_addr = b.addr;
        b.kill();
        let stopped = run_calls(&channel, 200).await;
        assert!(stopped.errors <= 20, "{stopped:?}");

        let b = Replica::start(b_addr);
        let rejoined = tokio::time::timeout(Duration::from_secs(10), async {
            while b.hits() == 0 {
                let stats = run_calls(&channel, 50).await;
                assert_eq!(stats.errors, 0, "{stats:?}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        assert!(
            rejoined.is_ok(),
            "a restarted replica must rejoin once its reconnect probe succeeds"
        );
    }

    /// TCP relay that can stop forwarding while keeping every socket open:
    /// the peer looks alive to the kernel but never answers, as when a killed
    /// container's address black-holes packets.
    struct FreezableRelay {
        addr: SocketAddr,
        frozen: Arc<std::sync::atomic::AtomicBool>,
    }

    impl FreezableRelay {
        async fn start(upstream: SocketAddr) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind relay");
            let addr = listener.local_addr().expect("relay address");
            let frozen = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = frozen.clone();
            tokio::spawn(async move {
                while let Ok((inbound, _)) = listener.accept().await {
                    let frozen = flag.clone();
                    tokio::spawn(async move {
                        if frozen.load(Ordering::Relaxed) {
                            let _held = inbound;
                            return std::future::pending::<()>().await;
                        }
                        let Ok(outbound) = tokio::net::TcpStream::connect(upstream).await else {
                            return;
                        };
                        let (inbound_read, inbound_write) = inbound.into_split();
                        let (outbound_read, outbound_write) = outbound.into_split();
                        tokio::join!(
                            pump(inbound_read, outbound_write, frozen.clone()),
                            pump(outbound_read, inbound_write, frozen),
                        );
                    });
                }
            });
            Self { addr, frozen }
        }

        fn freeze(&self) {
            self.frozen.store(true, Ordering::Relaxed);
        }
    }

    async fn pump(
        mut from: tokio::net::tcp::OwnedReadHalf,
        mut to: tokio::net::tcp::OwnedWriteHalf,
        frozen: Arc<std::sync::atomic::AtomicBool>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let read = match from.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => read,
            };
            if frozen.load(Ordering::Relaxed) {
                // Swallow traffic and keep both halves open.
                let _held = (from, to);
                std::future::pending::<()>().await;
                return;
            }
            if to.write_all(&buffer[..read]).await.is_err() {
                return;
            }
        }
    }

    /// A black-holed replica never returns a connection error, so isolation
    /// cannot kick in before HTTP/2 keepalive notices it. Until then P2C's
    /// in-flight load keeps most traffic away: only requests dispatched before
    /// its stuck count outgrew the healthy replica's load hit the deadline.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn black_holed_replica_is_avoided_by_in_flight_load() {
        let a = Replica::any_port();
        let b = Replica::any_port();
        let relay = FreezableRelay::start(b.addr).await;
        let backend = Arc::new(StaticBackend {
            instances: vec![
                ServiceInstance::new("test", "a", a.addr),
                ServiceInstance::new("test", "b", relay.addr),
            ],
        });
        let (discover, updater) = ServiceDiscover::from_backend(backend, config())
            .await
            .expect("discover");
        let channel = discover.into_balance_channel(None, REQUEST_TIMEOUT);

        let healthy = run_calls(&channel, CALLS).await;
        assert_eq!(healthy.errors, 0, "both replicas up: {healthy:?}");
        assert!(b.hits() > 0, "the relayed replica must take traffic");

        relay.freeze();
        let started = Instant::now();
        let frozen = run_calls(&channel, CALLS).await;
        let frozen_elapsed = started.elapsed();
        updater.remove("b").await;
        settle().await;
        let removed = run_calls(&channel, CALLS).await;

        println!(
            "BalancedChannel black-hole: healthy [{healthy:?}] frozen-without-Remove [{frozen:?}] wall={frozen_elapsed:?} after-Remove [{removed:?}]"
        );
        assert!(
            frozen.error_rate() <= 0.05,
            "in-flight load must steer traffic away from a silent replica: {frozen:?}"
        );
        assert_eq!(removed.errors, 0, "after Remove: {removed:?}");
    }

    /// Baseline for the black-hole case with tonic's own balance channel.
    /// Slow (every stuck call waits the full deadline), so run on demand:
    /// `cargo test -p flare-core-transport --features discovery -- --ignored --nocapture black_hole`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "slow baseline: stuck calls each wait the full deadline"]
    async fn tonic_balance_channel_black_hole_baseline() {
        let a = Replica::any_port();
        let b = Replica::any_port();
        let relay = FreezableRelay::start(b.addr).await;
        let (channel, tx) = Channel::balance_channel::<&'static str>(64);
        for (key, addr) in [("a", a.addr), ("b", relay.addr)] {
            let endpoint = Endpoint::from_shared(format!("http://{addr}"))
                .expect("endpoint")
                .timeout(REQUEST_TIMEOUT);
            tx.send(ChannelChange::Insert(key, endpoint))
                .await
                .expect("insert");
        }
        settle().await;
        let healthy = run_calls(&channel, 200).await;
        assert_eq!(healthy.errors, 0, "both replicas up: {healthy:?}");

        relay.freeze();
        let started = Instant::now();
        let frozen = run_calls(&channel, 200).await;
        let frozen_elapsed = started.elapsed();
        println!(
            "tonic balance_channel black-hole: healthy [{healthy:?}] frozen-without-Remove [{frozen:?}] wall={frozen_elapsed:?}"
        );
    }
}
