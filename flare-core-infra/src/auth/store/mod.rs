use std::time::Duration;

use anyhow::Result;

pub mod redis;

pub use redis::RedisTokenStore;

/// 令牌存储接口，用于记录活跃令牌并支持撤销
pub trait TokenStore: Send + Sync {
    /// 记录一个新签发的令牌
    fn register(&self, user_id: &str, jti: &str, ttl: Duration) -> Result<()>;

    /// 撤销指定令牌
    fn revoke(&self, user_id: &str, jti: &str, ttl: Duration) -> Result<()>;

    /// 撤销指定用户的全部令牌
    fn revoke_user(&self, user_id: &str, ttl: Duration) -> Result<()>;

    /// 检查令牌是否已被撤销
    fn is_revoked(&self, jti: &str) -> Result<bool>;

    /// 发布「踢用户」信号，通知各长连接网关关闭该用户的活连接（撤销即断）。
    /// 默认无操作；仅 redis 实现走 pubsub。
    fn publish_kick(&self, _user_id: &str) -> Result<()> {
        Ok(())
    }
}
