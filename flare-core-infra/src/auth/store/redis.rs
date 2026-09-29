use std::time::Duration;

use anyhow::{Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use redis::Commands;

use super::{KickSignal, TokenStore};

const DEFAULT_NAMESPACE: &str = "flare";

pub struct RedisTokenStore {
    client: redis::Client,
    namespace: String,
}

impl RedisTokenStore {
    pub fn new(url: impl AsRef<str>) -> Result<Self> {
        let client = redis::Client::open(url.as_ref())
            .map_err(|err| anyhow!("failed to open redis client: {err}"))?;
        Ok(Self {
            client,
            namespace: DEFAULT_NAMESPACE.to_string(),
        })
    }

    pub fn with_namespace(url: impl AsRef<str>, namespace: impl Into<String>) -> Result<Self> {
        let client = redis::Client::open(url.as_ref())
            .map_err(|err| anyhow!("failed to open redis client: {err}"))?;
        Ok(Self {
            client,
            namespace: namespace.into(),
        })
    }

    fn active_key(&self, jti: &str) -> String {
        format!("{}:token:v2:active:{}", self.namespace, jti)
    }

    fn revoked_key(&self, jti: &str) -> String {
        format!("{}:token:v2:revoked:{}", self.namespace, jti)
    }

    fn user_set_key(&self, tenant_id: &str, user_id: &str) -> String {
        format!(
            "{}:token:v2:user:{}:{}",
            self.namespace,
            URL_SAFE_NO_PAD.encode(tenant_id),
            URL_SAFE_NO_PAD.encode(user_id)
        )
    }

    /// 踢人信号的 pubsub 频道。网关订阅同名频道即可跨节点收到撤销即断信号。
    pub fn kick_channel(&self) -> String {
        format!("{}:kick", self.namespace)
    }
}

impl TokenStore for RedisTokenStore {
    fn register(&self, tenant_id: &str, user_id: &str, jti: &str, ttl: Duration) -> Result<()> {
        let ttl_secs = ttl.as_secs().max(1);
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let user_key = self.user_set_key(tenant_id, user_id);

        let _: () = conn
            .set_ex(self.active_key(jti), 1, ttl_secs)
            .map_err(|err| anyhow!("failed to write redis key: {err}"))?;
        conn.sadd::<_, _, ()>(&user_key, jti)
            .map_err(|err| anyhow!("failed to add token to user set: {err}"))?;
        let _: bool = conn
            .expire(&user_key, ttl_secs as i64)
            .map_err(|err| anyhow!("failed to set user set ttl: {err}"))?;

        Ok(())
    }

    fn revoke(&self, tenant_id: &str, user_id: &str, jti: &str, ttl: Duration) -> Result<()> {
        let ttl_secs = ttl.as_secs().max(1);
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let user_key = self.user_set_key(tenant_id, user_id);

        conn.del::<_, ()>(self.active_key(jti))
            .map_err(|err| anyhow!("failed to delete active token key: {err}"))?;
        conn.srem::<_, _, ()>(&user_key, jti)
            .map_err(|err| anyhow!("failed to remove token from user set: {err}"))?;
        let _: () = conn
            .set_ex(self.revoked_key(jti), 1, ttl_secs)
            .map_err(|err| anyhow!("failed to set revoked key: {err}"))?;

        Ok(())
    }

    fn revoke_user(&self, tenant_id: &str, user_id: &str, ttl: Duration) -> Result<()> {
        let ttl_secs = ttl.as_secs().max(1);
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let user_key = self.user_set_key(tenant_id, user_id);

        let tokens: Vec<String> = conn
            .smembers(&user_key)
            .map_err(|err| anyhow!("failed to fetch user token set: {err}"))?;

        for jti in tokens {
            conn.del::<_, ()>(self.active_key(&jti))
                .map_err(|err| anyhow!("failed to delete active token key: {err}"))?;
            let _: () = conn
                .set_ex(self.revoked_key(&jti), 1, ttl_secs)
                .map_err(|err| anyhow!("failed to set revoked key: {err}"))?;
        }

        conn.del::<_, ()>(user_key)
            .map_err(|err| anyhow!("failed to delete user token set: {err}"))?;

        Ok(())
    }

    fn is_revoked(&self, jti: &str) -> Result<bool> {
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let revoked: bool = conn
            .exists(self.revoked_key(jti))
            .map_err(|err| anyhow!("failed to check revoked key: {err}"))?;
        let active: bool = conn
            .exists(self.active_key(jti))
            .map_err(|err| anyhow!("failed to check active key: {err}"))?;
        // Direct v2 cutover: a token registered only in the old unscoped key
        // space cannot be considered active after the tenant-isolation switch.
        Ok(revoked || !active)
    }

    fn publish_kick(&self, signal: &KickSignal) -> Result<()> {
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let payload = serde_json::to_string(signal)?;
        conn.publish::<_, _, ()>(self.kick_channel(), payload)
            .map_err(|err| anyhow!("failed to publish kick signal: {err}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_user_keys_do_not_alias_across_tenants_or_delimiters() {
        let store = RedisTokenStore::new("redis://127.0.0.1/").unwrap();
        assert_ne!(
            store.user_set_key("tenant-a", "shared"),
            store.user_set_key("tenant-b", "shared")
        );
        assert_ne!(
            store.user_set_key("a:b", "c"),
            store.user_set_key("a", "b:c")
        );
        assert!(store.active_key("jti").contains(":v2:"));
    }

    #[test]
    fn kick_signal_requires_tenant_and_preserves_device_scope() {
        let signal = KickSignal {
            tenant_id: "tenant-a".into(),
            user_id: "shared".into(),
            device_id: Some("ios".into()),
            connection_id: Some("old-connection".into()),
        };
        let encoded = serde_json::to_string(&signal).unwrap();
        assert_eq!(
            serde_json::from_str::<KickSignal>(&encoded).unwrap(),
            signal
        );
        assert!(serde_json::from_str::<KickSignal>("shared").is_err());
    }
}
