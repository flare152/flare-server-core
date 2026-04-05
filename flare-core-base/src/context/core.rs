//! 核心上下文：单一 Context 结构体
//!
//! 与 metadata.proto 主链路对齐，含取消/超时/父子/自定义数据。Clone 为 Arc 引用计数。

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{Duration as TokioDuration, Instant as TokioInstant, sleep_until, timeout};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::typemap::TypeMap;

/// 任务控制：取消 + 超时（毫秒）
#[derive(Debug)]
pub struct TaskControl {
    pub cancel_token: CancellationToken,
    pub timeout_ms: Option<u64>,
}

/// 上下文：主链路元数据 + 取消/超时 + 父子 + TypeMap。
///
/// 内部为 `Arc` 包装，Clone 仅增加引用计数。
#[derive(Clone, Debug)]
pub struct Context {
    inner: Arc<ContextInner>,
}

pub type Ctx = Arc<Context>;

#[derive(Debug)]
struct ContextInner {
    cancel_token: CancellationToken,
    deadline: Option<Instant>,
    parent: Option<Arc<ContextInner>>,
    request_id: Arc<str>,
    trace_id: Arc<str>,
    user_id: Option<Arc<str>>,
    tenant_id: Option<Arc<str>>,
    device_id: Option<Arc<str>>,
    platform: Option<Arc<str>>,
    session_id: Option<Arc<str>>,
    data: TypeMap,
}

fn empty_arc() -> Arc<str> {
    Arc::from("")
}

impl Context {
    pub fn root() -> Self {
        Self {
            inner: Arc::new(ContextInner {
                cancel_token: CancellationToken::new(),
                deadline: None,
                parent: None,
                request_id: empty_arc(),
                trace_id: empty_arc(),
                user_id: None,
                tenant_id: None,
                device_id: None,
                platform: None,
                session_id: None,
                data: TypeMap::new(),
            }),
        }
    }

    pub fn with_request_id(request_id: impl AsRef<str>) -> Self {
        let r = request_id.as_ref();
        Self {
            inner: Arc::new(ContextInner {
                cancel_token: CancellationToken::new(),
                deadline: None,
                parent: None,
                request_id: if r.is_empty() {
                    empty_arc()
                } else {
                    Arc::from(r)
                },
                trace_id: empty_arc(),
                user_id: None,
                tenant_id: None,
                device_id: None,
                platform: None,
                session_id: None,
                data: TypeMap::new(),
            }),
        }
    }

    pub fn child(&self) -> Self {
        let child_token = self.inner.cancel_token.child_token();
        Self {
            inner: Arc::new(ContextInner {
                cancel_token: child_token,
                deadline: self.inner.deadline,
                parent: Some(self.inner.clone()),
                request_id: Arc::clone(&self.inner.request_id),
                trace_id: Arc::clone(&self.inner.trace_id),
                user_id: self.inner.user_id.clone(),
                tenant_id: self.inner.tenant_id.clone(),
                device_id: self.inner.device_id.clone(),
                platform: self.inner.platform.clone(),
                session_id: self.inner.session_id.clone(),
                data: self.inner.data.clone(),
            }),
        }
    }

    pub fn with_timeout(&self, d: Duration) -> Self {
        self.with_deadline(Instant::now() + d)
    }

    pub fn with_timeout_ms(&self, ms: u64) -> Self {
        self.with_timeout(Duration::from_millis(ms))
    }

    pub fn with_deadline(&self, deadline: Instant) -> Self {
        let child = self.child();
        let effective = match child.inner.deadline {
            Some(d) if d < deadline => d,
            _ => deadline,
        };
        if effective <= Instant::now() {
            child.inner.cancel_token.cancel();
        } else {
            let token = child.inner.cancel_token.clone();
            tokio::spawn(async move {
                sleep_until(TokioInstant::from_std(effective)).await;
                token.cancel();
            });
        }
        let mut inner = (*child.inner).clone();
        inner.deadline = Some(effective);
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn with_task(&self, timeout_ms: Option<u64>) -> Self {
        match timeout_ms {
            Some(ms) => self.with_timeout_ms(ms),
            None => self.child(),
        }
    }

    pub fn with_trace_id(&self, trace_id: impl AsRef<str>) -> Self {
        let c = self.child();
        Self {
            inner: Arc::new(ContextInner {
                trace_id: Arc::from(trace_id.as_ref()),
                ..(*c.inner).clone()
            }),
        }
    }

    pub fn with_user_id(&self, user_id: impl AsRef<str>) -> Self {
        let c = self.child();
        Self {
            inner: Arc::new(ContextInner {
                user_id: Some(Arc::from(user_id.as_ref())),
                ..(*c.inner).clone()
            }),
        }
    }

    pub fn with_tenant_id(&self, tenant_id: impl AsRef<str>) -> Self {
        let c = self.child();
        Self {
            inner: Arc::new(ContextInner {
                tenant_id: Some(Arc::from(tenant_id.as_ref())),
                ..(*c.inner).clone()
            }),
        }
    }

    pub fn with_device_id(&self, device_id: impl AsRef<str>) -> Self {
        let c = self.child();
        Self {
            inner: Arc::new(ContextInner {
                device_id: Some(Arc::from(device_id.as_ref())),
                ..(*c.inner).clone()
            }),
        }
    }

    pub fn with_platform(&self, platform: impl AsRef<str>) -> Self {
        let c = self.child();
        Self {
            inner: Arc::new(ContextInner {
                platform: Some(Arc::from(platform.as_ref())),
                ..(*c.inner).clone()
            }),
        }
    }

    pub fn with_session_id(&self, session_id: impl AsRef<str>) -> Self {
        let c = self.child();
        Self {
            inner: Arc::new(ContextInner {
                session_id: Some(Arc::from(session_id.as_ref())),
                ..(*c.inner).clone()
            }),
        }
    }

    pub fn insert_data<T: Send + Sync + 'static>(&self, value: T) -> Self {
        let mut data = self.inner.data.clone();
        data.insert(value);
        Self {
            inner: Arc::new(ContextInner {
                data,
                ..(*self.inner).clone()
            }),
        }
    }

    pub fn get_data<T: 'static>(&self) -> Option<&T> {
        self.inner.data.get::<T>()
    }

    pub fn remove_data<T: Send + Sync + 'static>(&self) -> (Self, Option<T>) {
        let mut data = self.inner.data.clone();
        let out = data.remove::<T>();
        (
            Self {
                inner: Arc::new(ContextInner {
                    data,
                    ..(*self.inner).clone()
                }),
            },
            out,
        )
    }

    pub fn with_actor(&self, actor: super::ActorContext) -> Self {
        self.insert_data(actor)
    }
    pub fn actor(&self) -> Option<&super::ActorContext> {
        self.get_data::<super::ActorContext>()
    }
    pub fn with_audit(&self, audit: super::AuditContext) -> Self {
        self.insert_data(audit)
    }
    pub fn audit(&self) -> Option<&super::AuditContext> {
        self.get_data::<super::AuditContext>()
    }

    pub fn cancel(&self) {
        self.inner.cancel_token.cancel();
        debug!(request_id = %self.inner.request_id, "Context cancelled");
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancel_token.is_cancelled()
    }

    pub async fn cancelled(&self) {
        self.inner.cancel_token.cancelled().await;
    }

    pub async fn run<F, T>(&self, fut: F) -> Option<T>
    where
        F: std::future::Future<Output = T>,
    {
        let token = self.inner.cancel_token.clone();
        let fut = async move {
            tokio::select! {
                r = fut => Some(r),
                _ = token.cancelled() => None,
            }
        };
        match self.inner.deadline {
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return None;
                }
                timeout(TokioDuration::from_secs_f64(left.as_secs_f64()), fut)
                    .await
                    .unwrap_or(None)
            }
            None => fut.await,
        }
    }

    pub fn remaining_time(&self) -> Option<Duration> {
        self.inner
            .deadline
            .filter(|d| *d > Instant::now())
            .map(|d| d.duration_since(Instant::now()))
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.inner.deadline
    }

    #[inline]
    pub fn request_id(&self) -> &str {
        &self.inner.request_id
    }
    #[inline]
    pub fn trace_id(&self) -> &str {
        &self.inner.trace_id
    }
    pub fn user_id(&self) -> Option<&str> {
        self.inner.user_id.as_deref()
    }
    pub fn tenant_id(&self) -> Option<&str> {
        self.inner.tenant_id.as_deref()
    }
    pub fn device_id(&self) -> Option<&str> {
        self.inner.device_id.as_deref()
    }
    pub fn platform(&self) -> Option<&str> {
        self.inner.platform.as_deref()
    }
    pub fn session_id(&self) -> Option<&str> {
        self.inner.session_id.as_deref()
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::root()
    }
}

/// 使 `&Context` 可直接用于 `set_context_metadata(request, ctx)` 等需要 `AsRef<Context>` 的 API。
impl AsRef<Context> for Context {
    fn as_ref(&self) -> &Context {
        self
    }
}

impl Clone for ContextInner {
    fn clone(&self) -> Self {
        Self {
            cancel_token: self.cancel_token.clone(),
            deadline: self.deadline,
            parent: self.parent.clone(),
            request_id: Arc::clone(&self.request_id),
            trace_id: Arc::clone(&self.trace_id),
            user_id: self.user_id.clone(),
            tenant_id: self.tenant_id.clone(),
            device_id: self.device_id.clone(),
            platform: self.platform.clone(),
            session_id: self.session_id.clone(),
            data: self.data.clone(),
        }
    }
}

pub trait ContextExt {
    fn ensure_not_cancelled(&self) -> Result<(), ContextError>;
    fn check_cancelled(&self) -> Result<(), ContextError>;
}

impl ContextExt for Context {
    fn ensure_not_cancelled(&self) -> Result<(), ContextError> {
        if self.is_cancelled() {
            Err(ContextError::Cancelled)
        } else {
            Ok(())
        }
    }
    fn check_cancelled(&self) -> Result<(), ContextError> {
        self.ensure_not_cancelled()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    #[error("context cancelled")]
    Cancelled,
    #[error("context deadline exceeded")]
    DeadlineExceeded,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_root() {
        let ctx = Context::root();
        assert!(!ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancel() {
        let ctx = Context::root();
        ctx.cancel();
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_child_inherits_cancel() {
        let parent = Context::root();
        let child = parent.child();
        parent.cancel();
        sleep(Duration::from_millis(10)).await;
        assert!(child.is_cancelled());
    }

    #[tokio::test]
    async fn test_with_timeout() {
        let ctx = Context::root().with_timeout(Duration::from_millis(50));
        sleep(Duration::from_millis(100)).await;
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancelled_future() {
        let ctx = Context::root();
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            ctx2.cancel();
        });
        ctx.cancelled().await;
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_remaining_time() {
        let ctx = Context::root().with_timeout(Duration::from_secs(10));
        let r = ctx.remaining_time().unwrap();
        assert!(r <= Duration::from_secs(10) && r > Duration::from_secs(9));
    }

    #[tokio::test]
    async fn test_metadata() {
        let ctx = Context::with_request_id("req-1")
            .with_trace_id("trace-1")
            .with_user_id("u1")
            .with_tenant_id("t1");
        assert_eq!(ctx.request_id(), "req-1");
        assert_eq!(ctx.trace_id(), "trace-1");
        assert_eq!(ctx.user_id(), Some("u1"));
        assert_eq!(ctx.tenant_id(), Some("t1"));
    }

    #[tokio::test]
    async fn test_run_ok() {
        let ctx = Context::root();
        assert_eq!(ctx.run(async { 42 }).await, Some(42));
    }

    #[tokio::test]
    async fn test_metadata_getters() {
        let ctx = Context::with_request_id("r")
            .with_trace_id("t")
            .with_user_id("u");
        assert_eq!(ctx.request_id(), "r");
        assert_eq!(ctx.trace_id(), "t");
        assert_eq!(ctx.user_id(), Some("u"));
    }
}
