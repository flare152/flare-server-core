use std::time::Duration;

use anyhow::{Result, anyhow};
use redis::Commands;

use super::TokenStore;

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
        format!("{}:token:active:{}", self.namespace, jti)
    }

    fn revoked_key(&self, jti: &str) -> String {
        format!("{}:token:revoked:{}", self.namespace, jti)
    }

    fn user_set_key(&self, user_id: &str) -> String {
        format!("{}:token:user:{}", self.namespace, user_id)
    }
}

impl TokenStore for RedisTokenStore {
    fn register(&self, user_id: &str, jti: &str, ttl: Duration) -> Result<()> {
        let ttl_secs = ttl.as_secs().max(1);
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let user_key = self.user_set_key(user_id);

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

    fn revoke(&self, user_id: &str, jti: &str, ttl: Duration) -> Result<()> {
        let ttl_secs = ttl.as_secs().max(1);
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let user_key = self.user_set_key(user_id);

        conn.del::<_, ()>(self.active_key(jti))
            .map_err(|err| anyhow!("failed to delete active token key: {err}"))?;
        conn.srem::<_, _, ()>(&user_key, jti)
            .map_err(|err| anyhow!("failed to remove token from user set: {err}"))?;
        let _: () = conn
            .set_ex(self.revoked_key(jti), 1, ttl_secs)
            .map_err(|err| anyhow!("failed to set revoked key: {err}"))?;

        Ok(())
    }

    fn revoke_user(&self, user_id: &str, ttl: Duration) -> Result<()> {
        let ttl_secs = ttl.as_secs().max(1);
        let mut conn = self
            .client
            .get_connection()
            .map_err(|err| anyhow!("failed to get redis connection: {err}"))?;
        let user_key = self.user_set_key(user_id);

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
        let exists: bool = conn
            .exists(self.revoked_key(jti))
            .map_err(|err| anyhow!("failed to check revoked key: {err}"))?;
        Ok(exists)
    }
}
