//! 随服务发现变化的 gRPC 负载均衡通道，连接级失败的节点会被隔离。
//!
//! tonic 自带的 `Channel::balance_channel` 给每个节点建懒连接：建连失败只把错误
//! 留给下一次 `call`，`poll_ready` 照样报 Ready。P2C 据此认为死副本仍可用，大约
//! 一半请求会落到它身上，直到服务发现发出 Remove 为止。
//!
//! [`BalancedChannel`] 自己组装 P2C：每个节点外包一层隔离适配器。某次调用出现
//! 连接级失败（建连被拒、连接断开等，不含单次请求超时）后，该节点的 `poll_ready`
//! 改为 Pending，并按指数退避重新建连探测，探测成功才重新参与选择。
//!
//! 这里有意不让 `poll_ready` 返回 Err：tower 的 P2C 会把报错的节点永久丢弃，只有
//! 同一 key 的新 Insert 才能带回来；而服务发现只在实例变化时发 Insert，同一实例 ID
//! 原地重启的副本就再也回不来了。Pending 让节点留在集合里，由探测决定何时恢复；
//! Remove 仍然照常生效。
//!
//! 负载指标是每个节点的在途请求数，P2C 因此会避开积压的节点（例如网络黑洞里、
//! 请求迟迟没有响应的副本）。

use std::convert::Infallible;
use std::fmt;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use futures::Stream;
use tokio::sync::mpsc;
use tonic::body::Body;
use tonic::transport::channel::Change as EndpointChange;
use tonic::transport::{Channel, Endpoint};
use tower::Service;
use tower::balance::p2c::Balance;
use tower::buffer::Buffer;
use tower::discover::Change;
use tower::load::{CompleteOnResponse, PendingRequests};
use tower::util::BoxService;

type BoxError = tower::BoxError;
type Request = http::Request<Body>;
type Response = http::Response<Body>;
type BoxedService = BoxService<Request, Response, BoxError>;
type BoxedFuture = <BoxedService as Service<Request>>::Future;

/// 与 tonic 一致的请求缓冲容量。
const BUFFER_CAPACITY: usize = 1024;
/// 隔离后第一次重新探测前的等待；每次探测失败翻倍，直到上限。
const MIN_PROBE_BACKOFF: Duration = Duration::from_millis(50);
const MAX_PROBE_BACKOFF: Duration = Duration::from_secs(2);

/// 可克隆的负载均衡 gRPC 通道，可直接作为 tonic 生成客户端的传输层。
///
/// 所有克隆共享同一个后台 worker；全部克隆释放后，worker 与节点集合随之释放，
/// [`BalancedChannel::balance_channel`] 返回的 Sender 也会观察到关闭。
#[derive(Clone)]
pub struct BalancedChannel {
    inner: Buffer<Request, BoxedFuture>,
}

impl fmt::Debug for BalancedChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BalancedChannel").finish_non_exhaustive()
    }
}

impl BalancedChannel {
    /// 建一个节点集合由 Sender 驱动的通道，用法与 tonic 的
    /// `Channel::balance_channel` 相同。
    ///
    /// `probe_timeout` 限定单次建连探测（TCP 与 HTTP/2 握手）的时长，节点在
    /// 探测成功之前不会接到请求。没有可用节点时请求会一直排队，调用方须自带
    /// 截止时间。必须在 Tokio 运行时内调用。
    pub fn balance_channel<K>(
        capacity: usize,
        probe_timeout: Duration,
    ) -> (Self, mpsc::Sender<EndpointChange<K, Endpoint>>)
    where
        K: Hash + Eq + Clone + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(capacity);
        let balance = Balance::new(EndpointChanges { rx, probe_timeout });
        let (inner, worker) = Buffer::pair(BoxService::new(balance), BUFFER_CAPACITY);
        tokio::spawn(worker);
        (Self { inner }, tx)
    }
}

impl Service<Request> for BalancedChannel {
    type Response = Response;
    type Error = BoxError;
    type Future = tower::buffer::future::ResponseFuture<BoxedFuture>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        self.inner.call(request)
    }
}

type TrackedEndpoint = PendingRequests<IsolatingEndpoint, CompleteOnResponse>;

/// 把 Endpoint 级变化转成 P2C 需要的服务变化。
struct EndpointChanges<K> {
    rx: mpsc::Receiver<EndpointChange<K, Endpoint>>,
    probe_timeout: Duration,
}

impl<K> Stream for EndpointChanges<K> {
    type Item = Result<Change<K, TrackedEndpoint>, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let probe_timeout = self.probe_timeout;
        self.rx.poll_recv(cx).map(|change| {
            change.map(|change| {
                Ok(match change {
                    EndpointChange::Insert(key, endpoint) => Change::Insert(
                        key,
                        PendingRequests::new(
                            IsolatingEndpoint::new(endpoint, probe_timeout),
                            CompleteOnResponse::default(),
                        ),
                    ),
                    EndpointChange::Remove(key) => Change::Remove(key),
                })
            })
        })
    }
}

type ConnectFuture = Pin<Box<dyn Future<Output = Result<Channel, BoxError>> + Send>>;

enum EndpointState {
    Connecting(ConnectFuture),
    Backoff(Pin<Box<tokio::time::Sleep>>),
    Ready {
        channel: Channel,
        // 每条连接一个标记：旧连接上迟到的失败不会波及新连接。
        failed: Arc<AtomicBool>,
    },
}

/// 单个节点的隔离适配器，见模块文档。
struct IsolatingEndpoint {
    endpoint: Endpoint,
    probe_timeout: Duration,
    backoff: Duration,
    state: EndpointState,
}

impl IsolatingEndpoint {
    fn new(endpoint: Endpoint, probe_timeout: Duration) -> Self {
        let state = connect(&endpoint, probe_timeout);
        Self {
            endpoint,
            probe_timeout,
            backoff: MIN_PROBE_BACKOFF,
            state,
        }
    }

    fn start_probe(&mut self) {
        self.state = connect(&self.endpoint, self.probe_timeout);
    }
}

fn connect(endpoint: &Endpoint, probe_timeout: Duration) -> EndpointState {
    let endpoint = endpoint.clone();
    EndpointState::Connecting(Box::pin(async move {
        match tokio::time::timeout(probe_timeout, endpoint.connect()).await {
            Ok(result) => result.map_err(Into::into),
            Err(_) => Err(format!("connect to {} timed out", endpoint.uri()).into()),
        }
    }))
}

/// 单次请求超时不算连接级失败：慢请求不代表连接坏了；网络黑洞里的节点靠
/// 在途请求数负载与 HTTP/2 keepalive 识别。
fn is_connection_failure(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(error) = source {
        if error.is::<tonic::TimeoutExpired>() {
            return false;
        }
        source = error.source();
    }
    true
}

impl Service<Request> for IsolatingEndpoint {
    type Response = Response;
    type Error = BoxError;
    type Future = Pin<Box<dyn Future<Output = Result<Response, BoxError>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        loop {
            match &mut self.state {
                EndpointState::Ready { channel, failed } => {
                    if failed.load(Ordering::Acquire) {
                        tracing::debug!(
                            endpoint = %self.endpoint.uri(),
                            "isolating endpoint after connection failure"
                        );
                        self.start_probe();
                        continue;
                    }
                    match channel.poll_ready(cx) {
                        Poll::Ready(Ok(())) => return Poll::Ready(Ok(())),
                        Poll::Pending => return Poll::Pending,
                        // 返回 Err 会被 P2C 永久丢弃，改为隔离后重新探测。
                        Poll::Ready(Err(error)) => {
                            tracing::debug!(
                                endpoint = %self.endpoint.uri(),
                                %error,
                                "endpoint channel failed; probing"
                            );
                            self.start_probe();
                        }
                    }
                }
                EndpointState::Connecting(connecting) => match ready!(connecting.as_mut().poll(cx))
                {
                    Ok(channel) => {
                        self.backoff = MIN_PROBE_BACKOFF;
                        self.state = EndpointState::Ready {
                            channel,
                            failed: Arc::new(AtomicBool::new(false)),
                        };
                    }
                    Err(error) => {
                        let delay = self.backoff;
                        self.backoff = (delay * 2).min(MAX_PROBE_BACKOFF);
                        tracing::debug!(
                            endpoint = %self.endpoint.uri(),
                            %error,
                            retry_in_ms = delay.as_millis() as u64,
                            "endpoint probe failed"
                        );
                        self.state = EndpointState::Backoff(Box::pin(tokio::time::sleep(delay)));
                    }
                },
                EndpointState::Backoff(sleep) => {
                    ready!(sleep.as_mut().poll(cx));
                    self.start_probe();
                }
            }
        }
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let EndpointState::Ready { channel, failed } = &mut self.state else {
            return Box::pin(async { Err("endpoint called before it was ready".into()) });
        };
        let failed = failed.clone();
        let response = channel.call(request);
        Box::pin(async move {
            response.await.map_err(|error| {
                if is_connection_failure(&error) {
                    failed.store(true, Ordering::Release);
                }
                error.into()
            })
        })
    }
}
