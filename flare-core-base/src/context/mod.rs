//! 上下文模块（极简、0 拷贝）
//!
//! 仅保留与 metadata.proto 对齐的四种上下文：CoreContext、ExtendedContext、ActorContext、AuditContext。
//! 字符串与集合均用 `Arc` 共享，Clone 仅增加引用计数。

use std::collections::HashMap;
use std::sync::Arc;

// -----------------------------------------------------------------------------
// 上下文键常量（统一用于 gRPC Metadata 和 MQ Headers）
// -----------------------------------------------------------------------------

/// 上下文键常量
pub mod keys {
    /// 追踪 ID
    pub const TRACE_ID: &str = "x-trace-id";

    /// 用户 ID
    pub const USER_ID: &str = "x-user-id";

    /// 租户 ID
    pub const TENANT_ID: &str = "x-tenant-id";

    /// 设备 ID
    pub const DEVICE_ID: &str = "x-device-id";

    /// 平台
    pub const PLATFORM: &str = "x-platform";

    /// 会话 ID
    pub const SESSION_ID: &str = "x-session-id";

    /// 请求 ID
    pub const REQUEST_ID: &str = "x-request-id";

    /// Actor ID
    pub const ACTOR_ID: &str = "x-actor-id";

    /// Actor Type
    pub const ACTOR_TYPE: &str = "x-actor-type";

    /// 消息来源
    pub const SOURCE: &str = "x-source";

    /// 消息类型
    pub const MESSAGE_TYPE: &str = "x-message-type";

    /// 重试次数
    pub const RETRY_COUNT: &str = "x-retry-count";
}

// -----------------------------------------------------------------------------
// ExtendedContext（按需使用，不进主链路）
// -----------------------------------------------------------------------------

/// 扩展上下文（与 metadata.proto ExtendedContext 对齐）
///
/// 使用 `Arc` 共享 attributes，Clone 零拷贝。
#[derive(Clone, Debug, Default)]
pub struct ExtendedContext {
    pub attributes: Arc<HashMap<Arc<str>, Arc<str>>>,
}

impl ExtendedContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_attribute(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let mut m = HashMap::new();
        for (k, v) in self.attributes.iter() {
            m.insert(Arc::clone(k), Arc::clone(v));
        }
        m.insert(Arc::from(key.as_ref()), Arc::from(value.as_ref()));
        self.attributes = Arc::new(m);
        self
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(Arc::as_ref)
    }
}

impl PartialEq for ExtendedContext {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.attributes, &other.attributes)
            || self.attributes.as_ref() == other.attributes.as_ref()
    }
}
impl Eq for ExtendedContext {}

// -----------------------------------------------------------------------------
// ActorType
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActorType {
    #[default]
    Unspecified = 0,
    User = 1,
    Service = 2,
    TenantAdmin = 3,
    System = 4,
    Guest = 5,
}

impl From<i32> for ActorType {
    fn from(v: i32) -> Self {
        match v {
            1 => ActorType::User,
            2 => ActorType::Service,
            3 => ActorType::TenantAdmin,
            4 => ActorType::System,
            5 => ActorType::Guest,
            _ => ActorType::Unspecified,
        }
    }
}

impl From<ActorType> for i32 {
    fn from(v: ActorType) -> i32 {
        v as i32
    }
}

// -----------------------------------------------------------------------------
// ActorContext（鉴权与审计）
// -----------------------------------------------------------------------------

/// 操作者上下文（与 metadata.proto ActorContext 对齐）
///
/// 字段均用 Arc 共享，Clone 零拷贝。
#[derive(Clone, Debug)]
pub struct ActorContext {
    pub actor_id: Arc<str>,
    pub actor_type: ActorType,
    pub roles: Arc<[Arc<str>]>,
    pub attributes: Arc<HashMap<Arc<str>, Arc<str>>>,
}

impl Default for ActorContext {
    fn default() -> Self {
        Self {
            actor_id: Arc::from(""),
            actor_type: ActorType::Unspecified,
            roles: Arc::from([]),
            attributes: Arc::new(HashMap::new()),
        }
    }
}

impl ActorContext {
    pub fn new(actor_id: impl AsRef<str>) -> Self {
        Self {
            actor_id: Arc::from(actor_id.as_ref()),
            actor_type: ActorType::User,
            roles: Arc::from([]),
            attributes: Arc::new(HashMap::new()),
        }
    }

    pub fn with_type(mut self, actor_type: ActorType) -> Self {
        self.actor_type = actor_type;
        self
    }

    pub fn with_role(mut self, role: impl AsRef<str>) -> Self {
        let mut v: Vec<Arc<str>> = self.roles.iter().map(Arc::clone).collect();
        v.push(Arc::from(role.as_ref()));
        self.roles = Arc::from(v);
        self
    }

    pub fn with_attribute(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let mut m = HashMap::new();
        for (k, v) in self.attributes.iter() {
            m.insert(Arc::clone(k), Arc::clone(v));
        }
        m.insert(Arc::from(key.as_ref()), Arc::from(value.as_ref()));
        self.attributes = Arc::new(m);
        self
    }

    #[inline]
    pub fn actor_id(&self) -> &str {
        self.actor_id.as_ref()
    }

    #[inline]
    pub fn roles(&self) -> &[Arc<str>] {
        &self.roles
    }

    #[inline]
    pub fn attributes(&self) -> &HashMap<Arc<str>, Arc<str>> {
        &self.attributes
    }
}

impl PartialEq for ActorContext {
    fn eq(&self, other: &Self) -> bool {
        self.actor_id == other.actor_id
            && self.actor_type == other.actor_type
            && self.roles == other.roles
            && self.attributes.as_ref() == other.attributes.as_ref()
    }
}
impl Eq for ActorContext {}

// -----------------------------------------------------------------------------
// AuditContext（敏感操作记录）
// -----------------------------------------------------------------------------

/// 审计上下文（与 metadata.proto AuditContext 对齐）
///
/// actor 与 metadata 用 Arc 共享，Clone 零拷贝。
#[derive(Clone, Debug)]
pub struct AuditContext {
    pub actor: Arc<ActorContext>,
    pub operated_at: i64,
    pub reason: Arc<str>,
    pub metadata: Arc<HashMap<Arc<str>, Arc<str>>>,
}

impl Default for AuditContext {
    fn default() -> Self {
        Self {
            actor: Arc::new(ActorContext::default()),
            operated_at: 0,
            reason: Arc::from(""),
            metadata: Arc::new(HashMap::new()),
        }
    }
}

impl AuditContext {
    pub fn new(actor: ActorContext) -> Self {
        Self {
            actor: Arc::new(actor),
            operated_at: chrono::Utc::now().timestamp_millis(),
            reason: Arc::from(""),
            metadata: Arc::new(HashMap::new()),
        }
    }

    pub fn with_operated_at(mut self, ts: i64) -> Self {
        self.operated_at = ts;
        self
    }

    pub fn with_reason(mut self, reason: impl AsRef<str>) -> Self {
        self.reason = Arc::from(reason.as_ref());
        self
    }

    pub fn with_metadata_entry(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        let mut m = HashMap::new();
        for (k, v) in self.metadata.iter() {
            m.insert(Arc::clone(k), Arc::clone(v));
        }
        m.insert(Arc::from(key.as_ref()), Arc::from(value.as_ref()));
        self.metadata = Arc::new(m);
        self
    }
}

impl PartialEq for AuditContext {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.actor, &other.actor)
            && self.operated_at == other.operated_at
            && self.reason == other.reason
            && self.metadata.as_ref() == other.metadata.as_ref()
    }
}
impl Eq for AuditContext {}

// -----------------------------------------------------------------------------
// 核心 Context 系统（core.rs）
// -----------------------------------------------------------------------------

pub mod core;
pub mod typemap;

pub use core::{Context, ContextError, ContextExt, Ctx, TaskControl};
pub use typemap::TypeMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extended_context() {
        let ext = ExtendedContext::new().with_attribute("k", "v");
        assert_eq!(ext.get("k"), Some("v"));
    }

    #[test]
    fn test_actor_context() {
        let actor = ActorContext::new("user-1")
            .with_type(ActorType::User)
            .with_role("admin");
        assert_eq!(actor.actor_id(), "user-1");
        assert_eq!(actor.roles.len(), 1);
    }

    #[test]
    fn test_audit_context() {
        let actor = ActorContext::new("user-1");
        let audit = AuditContext::new(actor).with_reason("test");
        assert_eq!(audit.reason.as_ref(), "test");
    }
}
