use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod redis;

pub use redis::RedisTokenStore;

/// Tenant-scoped gateway disconnect command. Never accept a bare user ID as a
/// kick command: the same user ID may exist in multiple tenants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KickSignal {
    pub tenant_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// Exact gateway connection to close when an old session is replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
}

/// 令牌存储接口，用于记录活跃令牌并支持撤销
pub trait TokenStore: Send + Sync {
    /// 记录一个新签发的令牌
    fn register(&self, tenant_id: &str, user_id: &str, jti: &str, ttl: Duration) -> Result<()>;

    /// 撤销指定令牌
    fn revoke(&self, tenant_id: &str, user_id: &str, jti: &str, ttl: Duration) -> Result<()>;

    /// 撤销指定用户的全部令牌
    fn revoke_user(&self, tenant_id: &str, user_id: &str, ttl: Duration) -> Result<()>;

    /// 检查令牌是否已被撤销
    fn is_revoked(&self, jti: &str) -> Result<bool>;

    /// 发布「踢用户」信号，通知各长连接网关关闭该用户的活连接（撤销即断）。
    /// 默认无操作；仅 redis 实现走 pubsub。
    fn publish_kick(&self, _signal: &KickSignal) -> Result<()> {
        Ok(())
    }
}
