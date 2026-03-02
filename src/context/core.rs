//! 核心上下文系统
//!
//! 提供类型安全、显式传递的上下文系统，符合 Rust 最佳实践：
//! - 类型安全：不使用 HashMap<String, Any>，而是使用显式的字段和类型映射表
//! - 显式传递：不使用 TLS，所有 Context 必须显式传递
//! - 取消语义：基于 tokio::sync::watch::channel
//! - 超时支持：基于 tokio::time::Instant
//! - 父子关系：子 Context 继承父 Context 的取消和超时
//! - 自定义数据：支持存储任意 Send + Sync 类型的数据（包括数据库连接、服务实例等）
//!
//! # 设计原则
//!
//! 1. **类型安全**：所有元数据都是显式字段，编译时检查；自定义数据使用 TypeMap 实现类型安全存储
//! 2. **显式传递**：Context 作为函数参数传递，不使用隐式全局变量
//! 3. **零成本抽象**：使用 Arc 共享，Clone 成本低
//! 4. **异步友好**：与 tokio 生态完美集成
//! 5. **灵活扩展**：支持存储任意用户定义的数据，包括数据库连接、服务实例等
//!
//! # 使用示例
//!
//! ```rust,no_run
//! use flare_server_core::context::{Context, ContextExt};
//! use std::time::Duration;
//! use std::sync::Arc;
//! use sqlx::PgPool;
//!
//! // 创建根 Context
//! let root_ctx = Context::root();
//!
//! // 创建带超时的子 Context
//! let timeout_ctx = root_ctx.with_timeout(Duration::from_secs(5));
//!
//! // 存储自定义数据（如数据库连接）
//! let db_pool: Arc<PgPool> = get_db_pool();
//! let ctx = root_ctx.insert_data(db_pool);
//!
//! // 在异步函数中使用
//! async fn process_request(ctx: &Context) -> Result<()> {
//!     // 检查是否已取消
//!     ctx.ensure_not_cancelled()?;
//!
//!     // 获取自定义数据
//!     let pool: Option<&Arc<PgPool>> = ctx.get_data();
//!
//!     // 等待取消信号
//!     tokio::select! {
//!         result = do_work() => result,
//!         _ = ctx.cancelled() => Err(anyhow::anyhow!("cancelled")),
//!     }
//! }
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::time::{sleep_until, Instant as TokioInstant};
use tracing::debug;
use super::typemap::TypeMap;

/// 核心上下文结构
///
/// 提供取消、超时和请求元数据传递功能。
/// 使用 Arc 内部可变性，支持 Clone 和并发访问。
#[derive(Clone, Debug)]
pub struct Context {
    /// 内部数据（使用 Arc 共享）
    inner: Arc<Inner>,
}

/// 内部数据结构
#[derive(Debug)]
struct Inner {
    /// 取消信号（使用 watch channel 实现）
    cancel_tx: watch::Sender<bool>,
    /// 取消接收端
    cancel_rx: watch::Receiver<bool>,
    /// 父 Context（如果有）
    parent: Option<Arc<Inner>>,
    /// 截止时间（可选）
    deadline: Option<Instant>,
    /// 请求 ID（用于追踪）
    request_id: String,
    /// 追踪 ID（用于分布式追踪）
    trace_id: String,
    /// 用户 ID（可选）
    user_id: Option<String>,
    /// 租户 ID（可选）
    tenant_id: Option<String>,
    /// 会话 ID（可选）
    session_id: Option<String>,
    /// 自定义数据存储（类型安全的键值存储）
    data: TypeMap,
}

impl Context {
    /// 创建根 Context
    ///
    /// 根 Context 没有父 Context，取消操作只影响自身和子 Context。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    ///
    /// let root = Context::root();
    /// ```
    pub fn root() -> Self {
        let (tx, rx) = watch::channel(false);
        Self {
            inner: Arc::new(Inner {
                cancel_tx: tx,
                cancel_rx: rx,
                parent: None,
                deadline: None,
                request_id: String::new(),
                trace_id: String::new(),
                user_id: None,
                tenant_id: None,
                session_id: None,
                data: TypeMap::new(),
            }),
        }
    }

    /// 创建带请求 ID 的根 Context
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    ///
    /// let ctx = Context::with_request_id("req-123");
    /// ```
    pub fn with_request_id(request_id: impl Into<String>) -> Self {
        let (tx, rx) = watch::channel(false);
        Self {
            inner: Arc::new(Inner {
                cancel_tx: tx,
                cancel_rx: rx,
                parent: None,
                deadline: None,
                request_id: request_id.into(),
                trace_id: String::new(),
                user_id: None,
                tenant_id: None,
                session_id: None,
                data: TypeMap::new(),
            }),
        }
    }

    /// 创建子 Context
    ///
    /// 子 Context 继承父 Context 的取消令牌和截止时间（如果父 Context 的截止时间更早）。
    /// 子 Context 的取消不会影响父 Context。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    ///
    /// let parent = Context::root();
    /// let child = parent.child();
    /// ```
    pub fn child(&self) -> Self {
        // 子 Context 使用新的取消信号，但会监听父 Context 的取消
        let (child_tx, child_rx) = watch::channel(false);
        let parent_rx = self.inner.cancel_rx.clone();
        
        // 当父 Context 取消时，自动取消子 Context
        let child_tx_clone = child_tx.clone();
        tokio::spawn(async move {
            let mut parent_rx = parent_rx;
            loop {
                tokio::select! {
                    _ = parent_rx.changed() => {
                        if *parent_rx.borrow() {
                            let _ = child_tx_clone.send(true);
                            break;
                        }
                    }
                }
            }
        });

        // 选择更早的截止时间
        let deadline = match (self.inner.deadline, self.inner.parent.as_ref().and_then(|p| p.deadline)) {
            (Some(d1), Some(d2)) => Some(d1.min(d2)),
            (Some(d), None) | (None, Some(d)) => Some(d),
            (None, None) => None,
        };

        Self {
            inner: Arc::new(Inner {
                cancel_tx: child_tx,
                cancel_rx: child_rx,
                parent: Some(self.inner.clone()),
                deadline,
                request_id: self.inner.request_id.clone(),
                trace_id: self.inner.trace_id.clone(),
                user_id: self.inner.user_id.clone(),
                tenant_id: self.inner.tenant_id.clone(),
                session_id: self.inner.session_id.clone(),
                data: self.inner.data.clone(),
            }),
        }
    }

    /// 创建带超时的子 Context
    ///
    /// 如果父 Context 的截止时间更早，则使用父 Context 的截止时间。
    ///
    /// # 参数
    ///
    /// * `timeout` - 超时时间
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    /// use std::time::Duration;
    ///
    /// let parent = Context::root();
    /// let timeout_ctx = parent.with_timeout(Duration::from_secs(5));
    /// ```
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        let deadline = Instant::now() + timeout;
        self.with_deadline(deadline)
    }

    /// 创建带截止时间的子 Context
    ///
    /// 如果父 Context 的截止时间更早，则使用父 Context 的截止时间。
    ///
    /// # 参数
    ///
    /// * `deadline` - 截止时间
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    /// use std::time::{Duration, Instant};
    ///
    /// let parent = Context::root();
    /// let deadline = Instant::now() + Duration::from_secs(10);
    /// let deadline_ctx = parent.with_deadline(deadline);
    /// ```
    pub fn with_deadline(&self, deadline: Instant) -> Self {
        let child = self.child();
        
        // 选择更早的截止时间
        let effective_deadline = match child.inner.deadline {
            Some(existing) if existing < deadline => Some(existing),
            _ => Some(deadline),
        };

        // 如果截止时间已过期，立即取消
        if let Some(d) = effective_deadline {
            if d <= Instant::now() {
                let _ = child.inner.cancel_tx.send(true);
            } else {
                // 启动超时任务
                let cancel_tx = child.inner.cancel_tx.clone();
                tokio::spawn(async move {
                    sleep_until(TokioInstant::from_std(d)).await;
                    let _ = cancel_tx.send(true);
                });
            }
        }

        // 创建新的 Inner，保留 child 的其他字段
        // 注意：watch channel 不能直接 clone，需要创建新的 channel
        let (new_tx, new_rx) = watch::channel(*child.inner.cancel_rx.borrow());
        
        // 监听原 channel 的变化并转发到新 channel
        let old_rx = child.inner.cancel_rx.clone();
        let new_tx_clone = new_tx.clone();
        tokio::spawn(async move {
            let mut old_rx = old_rx;
            loop {
                tokio::select! {
                    _ = old_rx.changed() => {
                        if *old_rx.borrow() {
                            let _ = new_tx_clone.send(true);
                            break;
                        }
                    }
                }
            }
        });
        
        let new_inner = Arc::new(Inner {
            cancel_tx: new_tx,
            cancel_rx: new_rx,
            parent: child.inner.parent.clone(),
            deadline: effective_deadline,
            request_id: child.inner.request_id.clone(),
            trace_id: child.inner.trace_id.clone(),
            user_id: child.inner.user_id.clone(),
            tenant_id: child.inner.tenant_id.clone(),
            session_id: child.inner.session_id.clone(),
            data: child.inner.data.clone(),
        });
        
        Self { inner: new_inner }
    }

    /// 创建带追踪 ID 的子 Context
    pub fn with_trace_id(&self, trace_id: impl Into<String>) -> Self {
        let child = self.child();
        let trace_id = trace_id.into();
        let (tx, rx) = watch::channel(*child.inner.cancel_rx.borrow());
        
        // 转发取消信号
        let old_rx = child.inner.cancel_rx.clone();
        let new_tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut old_rx = old_rx;
            loop {
                tokio::select! {
                    _ = old_rx.changed() => {
                        if *old_rx.borrow() {
                            let _ = new_tx_clone.send(true);
                            break;
                        }
                    }
                }
            }
        });
        
        Self {
            inner: Arc::new(Inner {
                cancel_tx: tx,
                cancel_rx: rx,
                parent: child.inner.parent.clone(),
                deadline: child.inner.deadline,
                request_id: child.inner.request_id.clone(),
                trace_id,
                user_id: child.inner.user_id.clone(),
                tenant_id: child.inner.tenant_id.clone(),
                session_id: child.inner.session_id.clone(),
                data: child.inner.data.clone(),
            }),
        }
    }

    /// 创建带用户 ID 的子 Context
    pub fn with_user_id(&self, user_id: impl Into<String>) -> Self {
        let child = self.child();
        let user_id = Some(user_id.into());
        let (tx, rx) = watch::channel(*child.inner.cancel_rx.borrow());
        
        // 转发取消信号
        let old_rx = child.inner.cancel_rx.clone();
        let new_tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut old_rx = old_rx;
            loop {
                tokio::select! {
                    _ = old_rx.changed() => {
                        if *old_rx.borrow() {
                            let _ = new_tx_clone.send(true);
                            break;
                        }
                    }
                }
            }
        });
        
        Self {
            inner: Arc::new(Inner {
                cancel_tx: tx,
                cancel_rx: rx,
                parent: child.inner.parent.clone(),
                deadline: child.inner.deadline,
                request_id: child.inner.request_id.clone(),
                trace_id: child.inner.trace_id.clone(),
                user_id,
                tenant_id: child.inner.tenant_id.clone(),
                session_id: child.inner.session_id.clone(),
                data: child.inner.data.clone(),
            }),
        }
    }

    /// 创建带租户 ID 的子 Context
    pub fn with_tenant_id(&self, tenant_id: impl Into<String>) -> Self {
        let child = self.child();
        let tenant_id = Some(tenant_id.into());
        let (tx, rx) = watch::channel(*child.inner.cancel_rx.borrow());
        
        // 转发取消信号
        let old_rx = child.inner.cancel_rx.clone();
        let new_tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut old_rx = old_rx;
            loop {
                tokio::select! {
                    _ = old_rx.changed() => {
                        if *old_rx.borrow() {
                            let _ = new_tx_clone.send(true);
                            break;
                        }
                    }
                }
            }
        });
        
        Self {
            inner: Arc::new(Inner {
                cancel_tx: tx,
                cancel_rx: rx,
                parent: child.inner.parent.clone(),
                deadline: child.inner.deadline,
                request_id: child.inner.request_id.clone(),
                trace_id: child.inner.trace_id.clone(),
                user_id: child.inner.user_id.clone(),
                tenant_id,
                session_id: child.inner.session_id.clone(),
                data: child.inner.data.clone(),
            }),
        }
    }

    /// 创建带会话 ID 的子 Context
    pub fn with_session_id(&self, session_id: impl Into<String>) -> Self {
        let child = self.child();
        let session_id = Some(session_id.into());
        let (tx, rx) = watch::channel(*child.inner.cancel_rx.borrow());
        
        // 转发取消信号
        let old_rx = child.inner.cancel_rx.clone();
        let new_tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut old_rx = old_rx;
            loop {
                tokio::select! {
                    _ = old_rx.changed() => {
                        if *old_rx.borrow() {
                            let _ = new_tx_clone.send(true);
                            break;
                        }
                    }
                }
            }
        });
        
        Self {
            inner: Arc::new(Inner {
                cancel_tx: tx,
                cancel_rx: rx,
                parent: child.inner.parent.clone(),
                deadline: child.inner.deadline,
                request_id: child.inner.request_id.clone(),
                trace_id: child.inner.trace_id.clone(),
                user_id: child.inner.user_id.clone(),
                tenant_id: child.inner.tenant_id.clone(),
                session_id,
                data: child.inner.data.clone(),
            }),
        }
    }

    /// 存储自定义数据
    ///
    /// 支持存储任意 `Send + Sync` 类型的数据，包括数据库连接、服务实例等。
    ///
    /// # 示例
    ///
    /// ```rust,no_run
    /// use flare_server_core::context::Context;
    /// use sqlx::PgPool;
    ///
    /// let ctx = Context::root();
    /// let db_pool: Arc<PgPool> = get_db_pool();
    /// ctx.insert_data(db_pool);
    ///
    /// // 后续可以获取
    /// let pool: Option<&Arc<PgPool>> = ctx.get_data();
    /// ```
    pub fn insert_data<T: Send + Sync + 'static>(&self, value: T) -> Self {
        let mut new_data = self.inner.data.clone();
        new_data.insert(value);
        
        let new_inner = Arc::new(Inner {
            cancel_tx: self.inner.cancel_tx.clone(),
            cancel_rx: self.inner.cancel_rx.clone(),
            parent: self.inner.parent.clone(),
            deadline: self.inner.deadline,
            request_id: self.inner.request_id.clone(),
            trace_id: self.inner.trace_id.clone(),
            user_id: self.inner.user_id.clone(),
            tenant_id: self.inner.tenant_id.clone(),
            session_id: self.inner.session_id.clone(),
            data: new_data,
        });
        
        Self { inner: new_inner }
    }

    /// 获取自定义数据
    ///
    /// 返回指定类型的值的引用，如果不存在则返回 `None`。
    ///
    /// # 示例
    ///
    /// ```rust,no_run
    /// use flare_server_core::context::Context;
    /// use std::sync::Arc;
    /// use sqlx::PgPool;
    ///
    /// let ctx = Context::root();
    /// let pool: Option<&Arc<PgPool>> = ctx.get_data();
    /// ```
    pub fn get_data<T: 'static>(&self) -> Option<&T> {
        self.inner.data.get::<T>()
    }

    /// 移除自定义数据
    ///
    /// 移除指定类型的数据，如果存在则返回该值。
    /// 
    /// 注意：由于 Context 是不可变的，此方法会创建一个新的 Context。
    pub fn remove_data<T: Send + Sync + 'static>(&self) -> (Self, Option<T>) {
        let mut new_data = self.inner.data.clone();
        let removed = new_data.remove::<T>();
        
        let new_inner = Arc::new(Inner {
            cancel_tx: self.inner.cancel_tx.clone(),
            cancel_rx: self.inner.cancel_rx.clone(),
            parent: self.inner.parent.clone(),
            deadline: self.inner.deadline,
            request_id: self.inner.request_id.clone(),
            trace_id: self.inner.trace_id.clone(),
            user_id: self.inner.user_id.clone(),
            tenant_id: self.inner.tenant_id.clone(),
            session_id: self.inner.session_id.clone(),
            data: new_data,
        });
        
        (Self { inner: new_inner }, removed)
    }

    /// 设置租户上下文
    pub fn with_tenant(&self, tenant: super::TenantContext) -> Self {
        self.insert_data(tenant)
    }

    /// 获取租户上下文
    pub fn tenant(&self) -> Option<&super::TenantContext> {
        self.get_data::<super::TenantContext>()
    }

    /// 设置请求上下文
    pub fn with_request(&self, request: super::RequestContext) -> Self {
        self.insert_data(request)
    }

    /// 获取请求上下文
    pub fn request(&self) -> Option<&super::RequestContext> {
        self.get_data::<super::RequestContext>()
    }

    /// 设置追踪上下文
    pub fn with_trace(&self, trace: super::TraceContext) -> Self {
        self.insert_data(trace)
    }

    /// 获取追踪上下文
    pub fn trace(&self) -> Option<&super::TraceContext> {
        self.get_data::<super::TraceContext>()
    }

    /// 设置操作者上下文
    pub fn with_actor(&self, actor: super::ActorContext) -> Self {
        self.insert_data(actor)
    }

    /// 获取操作者上下文
    pub fn actor(&self) -> Option<&super::ActorContext> {
        self.get_data::<super::ActorContext>()
    }

    /// 设置设备上下文
    pub fn with_device(&self, device: super::DeviceContext) -> Self {
        self.insert_data(device)
    }

    /// 获取设备上下文
    pub fn device(&self) -> Option<&super::DeviceContext> {
        self.get_data::<super::DeviceContext>()
    }

    /// 设置审计上下文
    pub fn with_audit(&self, audit: super::AuditContext) -> Self {
        self.insert_data(audit)
    }

    /// 获取审计上下文
    pub fn audit(&self) -> Option<&super::AuditContext> {
        self.get_data::<super::AuditContext>()
    }

    /// 取消 Context
    ///
    /// 取消当前 Context 和所有子 Context。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    ///
    /// let ctx = Context::root();
    /// ctx.cancel();
    /// assert!(ctx.is_cancelled());
    /// ```
    pub fn cancel(&self) {
        let _ = self.inner.cancel_tx.send(true);
        debug!(request_id = %self.inner.request_id, "Context cancelled");
    }

    /// 检查是否已取消
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    ///
    /// let ctx = Context::root();
    /// assert!(!ctx.is_cancelled());
    /// ctx.cancel();
    /// assert!(ctx.is_cancelled());
    /// ```
    pub fn is_cancelled(&self) -> bool {
        *self.inner.cancel_rx.borrow()
    }

    /// 等待取消信号
    ///
    /// 返回一个 Future，当 Context 被取消时完成。
    ///
    /// # 示例
    ///
    /// ```rust,no_run
    /// use flare_server_core::context::core::Context;
    /// use tokio::select;
    ///
    /// # async fn example() {
    /// let ctx = Context::root();
    ///
    /// tokio::select! {
    ///     result = do_work() => {
    ///         // 工作完成
    ///     }
    ///     _ = ctx.cancelled() => {
    ///         // Context 被取消
    ///     }
    /// }
    /// # }
    /// # async fn do_work() {}
    /// ```
    pub async fn cancelled(&self) {
        let mut rx = self.inner.cancel_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// 获取剩余时间
    ///
    /// 返回 `None` 如果没有设置截止时间，否则返回剩余时间。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use flare_server_core::context::core::Context;
    /// use std::time::Duration;
    ///
    /// let ctx = Context::root().with_timeout(Duration::from_secs(10));
    /// if let Some(remaining) = ctx.remaining_time() {
    ///     println!("Remaining time: {:?}", remaining);
    /// }
    /// ```
    pub fn remaining_time(&self) -> Option<Duration> {
        self.inner.deadline.and_then(|deadline| {
            let now = Instant::now();
            if deadline > now {
                Some(deadline.duration_since(now))
            } else {
                None
            }
        })
    }

    /// 获取截止时间
    ///
    /// 返回 `None` 如果没有设置截止时间。
    pub fn deadline(&self) -> Option<Instant> {
        self.inner.deadline
    }

    /// 获取请求 ID
    pub fn request_id(&self) -> &str {
        &self.inner.request_id
    }

    /// 获取追踪 ID
    pub fn trace_id(&self) -> &str {
        &self.inner.trace_id
    }

    /// 获取用户 ID
    pub fn user_id(&self) -> Option<&str> {
        self.inner.user_id.as_deref()
    }

    /// 获取租户 ID
    pub fn tenant_id(&self) -> Option<&str> {
        self.inner.tenant_id.as_deref()
    }

    /// 获取会话 ID
    pub fn session_id(&self) -> Option<&str> {
        self.inner.session_id.as_deref()
    }
}

impl Default for Context {
    fn default() -> Self {
        Self::root()
    }
}

/// Context 扩展 trait
///
/// 提供便捷方法，用于在异步函数中处理 Context。
pub trait ContextExt {
    /// 检查是否已取消，如果已取消则返回错误
    ///
    /// # 示例
    ///
    /// ```rust,no_run
    /// use flare_server_core::context::core::{Context, ContextExt};
    ///
    /// # async fn example() -> Result<(), anyhow::Error> {
    /// let ctx = Context::root();
    /// ctx.ensure_not_cancelled()?;
    /// # Ok(())
    /// # }
    /// ```
    fn ensure_not_cancelled(&self) -> Result<(), ContextError>;

    /// 在 Context 取消时返回错误
    ///
    /// 用于在长时间运行的任务中定期检查取消状态。
    ///
    /// # 示例
    ///
    /// ```rust,no_run
    /// use flare_server_core::context::core::{Context, ContextExt};
    ///
    /// # async fn example() -> Result<(), anyhow::Error> {
    /// let ctx = Context::root();
    /// loop {
    ///     ctx.check_cancelled()?;
    ///     // 执行工作...
    /// }
    /// # Ok(())
    /// # }
    /// ```
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

/// Context 错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    /// Context 已被取消
    #[error("context cancelled")]
    Cancelled,
    /// Context 已超时
    #[error("context deadline exceeded")]
    DeadlineExceeded,
}

/// 从 RequestContext 创建 Context
///
/// 将现有的 RequestContext 转换为新的 Context 系统。
impl From<&super::RequestContext> for Context {
    fn from(req_ctx: &super::RequestContext) -> Self {
        let mut ctx = Context::with_request_id(&req_ctx.request_id);
        
        if let Some(trace) = &req_ctx.trace {
            ctx = ctx.with_trace_id(&trace.trace_id);
        }
        
        if let Some(actor) = &req_ctx.actor {
            ctx = ctx.with_user_id(&actor.actor_id);
        }
        
        ctx
    }
}

/// 从 Context 创建 RequestContext
///
/// 将新的 Context 转换为现有的 RequestContext。
impl From<&Context> for super::RequestContext {
    fn from(ctx: &Context) -> Self {
        let mut req_ctx = super::RequestContext::new(ctx.request_id());
        
        if !ctx.trace_id().is_empty() {
            req_ctx = req_ctx.with_trace(
                super::TraceContext::new(ctx.trace_id())
            );
        }
        
        if let Some(user_id) = ctx.user_id() {
            req_ctx = req_ctx.with_actor(
                super::ActorContext::new(user_id)
            );
        }
        
        req_ctx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_root_context() {
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
    async fn test_child_context() {
        let parent = Context::root();
        let child = parent.child();
        
        // 取消父 Context 应该取消子 Context
        parent.cancel();
        
        // 等待一小段时间确保取消传播
        sleep(Duration::from_millis(10)).await;
        
        assert!(parent.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[tokio::test]
    async fn test_timeout() {
        let ctx = Context::root().with_timeout(Duration::from_millis(50));
        
        // 等待超时
        sleep(Duration::from_millis(100)).await;
        
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancelled_future() {
        let ctx = Context::root();
        let cancel_tx = ctx.inner.cancel_tx.clone();
        
        tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            let _ = cancel_tx.send(true);
        });
        
        // 等待取消
        ctx.cancelled().await;
        assert!(ctx.is_cancelled());
    }

    #[tokio::test]
    async fn test_remaining_time() {
        let ctx = Context::root().with_timeout(Duration::from_secs(10));
        let remaining = ctx.remaining_time().unwrap();
        assert!(remaining <= Duration::from_secs(10));
        assert!(remaining > Duration::from_secs(9));
    }

    #[tokio::test]
    async fn test_with_metadata() {
            let ctx = Context::with_request_id("req-123")
                .with_trace_id("trace-456")
                .with_user_id("user-789")
                .with_tenant_id("tenant-abc");
        
        assert_eq!(ctx.request_id(), "req-123");
        assert_eq!(ctx.trace_id(), "trace-456");
        assert_eq!(ctx.user_id(), Some("user-789"));
        assert_eq!(ctx.tenant_id(), Some("tenant-abc"));
    }

    #[tokio::test]
    async fn test_select_with_context() {
        let ctx = Context::root();
        let cancel_tx = ctx.inner.cancel_tx.clone();
        
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            let _ = cancel_tx.send(true);
        });
        
        let mut work_done = false;
        tokio::select! {
            _ = async {
                sleep(Duration::from_secs(1)).await;
                work_done = true;
            } => {
                // 工作完成
            }
            _ = ctx.cancelled() => {
                // Context 被取消
            }
        }
        
        // 由于 Context 被取消，work_done 应该是 false
        assert!(!work_done);
        assert!(ctx.is_cancelled());
    }
}

