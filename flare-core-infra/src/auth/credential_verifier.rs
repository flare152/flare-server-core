//! Bounded local projection verifier. No Control calls, JWT/User fallback,
//! tenant activation, feature gates or resource authorization live here.
use super::{
    api_key::{ApiKeyPepper, ApiKeyToken},
    api_key_policy::{ApiKeyGrant, ApiKeyPolicyError, AuthorizedApiKey},
    credential_snapshot::CredentialKeyRecord,
};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore},
    time::{Instant, timeout},
};

const CAPACITY: usize = 4096;
const SHARDS: usize = 64;
const MAX_READS: usize = 16;
const POSITIVE_TTL: Duration = Duration::from_secs(1);
const NEGATIVE_TTL: Duration = Duration::from_millis(100);
const READ_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialReadError {
    #[error("local credential storage unavailable")]
    Unavailable,
    #[error("invalid local credential projection")]
    InvalidProjection,
}

/// A single primary-database statement must read selector, binding, payload
/// and version from the same committed snapshot. No raw credential material.
#[derive(Clone)]
pub struct LocalCredential {
    key_id: String,
    version: i64,
    grant: ApiKeyGrant,
}

impl std::fmt::Debug for LocalCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalCredential")
            .field("version", &self.version)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

impl LocalCredential {
    pub fn new(
        requested_key_id: &str,
        app_id: String,
        version: i64,
        record: CredentialKeyRecord,
    ) -> Result<Self, CredentialReadError> {
        if version <= 0 || record.key_id != requested_key_id {
            return Err(CredentialReadError::InvalidProjection);
        }
        let grant = record
            .grant(&app_id)
            .map_err(|_| CredentialReadError::InvalidProjection)?;
        Ok(Self {
            key_id: record.key_id,
            version,
            grant,
        })
    }
    pub fn version(&self) -> i64 {
        self.version
    }
}

#[async_trait]
pub trait LocalCredentialReader: Send + Sync {
    async fn lookup(&self, key_id: &str) -> Result<Option<LocalCredential>, CredentialReadError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialAuthError {
    #[error("API key authentication rejected")]
    Unauthenticated,
    #[error("API key authorization rejected")]
    Forbidden,
    #[error("API key verifier unavailable")]
    Unavailable,
}

struct Entry {
    value: Option<Arc<LocalCredential>>,
    deadline: Instant,
}

/// Cache grants, never tokens or successful identities. Every hit repeats the
/// digest/mode/pepper/scope/IP/expiry checks. There is no stale-on-error grace.
/// After local projection commit, an old positive may live at most 1 second;
/// this is NOT a promise about upstream delivery lag or tenant-state gates.
pub struct CachedApiKeyVerifier {
    reader: Arc<dyn LocalCredentialReader>,
    entries: Mutex<HashMap<String, Entry>>,
    flights: Vec<AsyncMutex<()>>,
    reads: Semaphore,
}

impl CachedApiKeyVerifier {
    pub fn new(reader: Arc<dyn LocalCredentialReader>) -> Self {
        Self {
            reader,
            entries: Mutex::new(HashMap::new()),
            flights: (0..SHARDS).map(|_| AsyncMutex::new(())).collect(),
            reads: Semaphore::new(MAX_READS),
        }
    }

    // Outer Option distinguishes miss from a cached negative. Never cache by
    // Authorization text (including secret), app supplied by the caller or IP.
    fn cached(
        &self,
        key_id: &str,
    ) -> Result<Option<Option<Arc<LocalCredential>>>, CredentialAuthError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialAuthError::Unavailable)?;
        if let Some(entry) = entries.get(key_id) {
            if Instant::now() < entry.deadline {
                return Ok(Some(entry.value.clone()));
            }
            entries.remove(key_id);
        }
        Ok(None)
    }

    async fn local(
        &self,
        key_id: &str,
    ) -> Result<Option<Arc<LocalCredential>>, CredentialAuthError> {
        if let Some(value) = self.cached(key_id)? {
            return Ok(value);
        }
        // Try-acquire bounds both IO and shard waiters. Fixed shard locks avoid
        // attacker-selected IDs growing an unbounded single-flight map.
        let _permit = self
            .reads
            .try_acquire()
            .map_err(|_| CredentialAuthError::Unavailable)?;
        let shard = key_id
            .bytes()
            .fold(0usize, |n, b| n.wrapping_mul(31).wrapping_add(b as usize))
            % SHARDS;
        let _flight = timeout(READ_TIMEOUT, self.flights[shard].lock())
            .await
            .map_err(|_| CredentialAuthError::Unavailable)?;
        if let Some(value) = self.cached(key_id)? {
            return Ok(value);
        }
        let started = Instant::now();
        let value = timeout(READ_TIMEOUT, self.reader.lookup(key_id))
            .await
            .map_err(|_| CredentialAuthError::Unavailable)?
            .map_err(|_| CredentialAuthError::Unavailable)?;
        if value.as_ref().is_some_and(|v| v.key_id != key_id) {
            return Err(CredentialAuthError::Unavailable);
        }
        let ttl = if value.is_some() {
            POSITIVE_TTL
        } else {
            NEGATIVE_TTL
        };
        // TTL begins before IO: a slow old snapshot cannot gain another full
        // second after a revocation commits while its read is in progress.
        let deadline = started + ttl;
        if Instant::now() >= deadline {
            return Err(CredentialAuthError::Unavailable);
        }
        let value = value.map(Arc::new);
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CredentialAuthError::Unavailable)?;
        entries.retain(|_, entry| Instant::now() < entry.deadline);
        if entries.len() >= CAPACITY {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.deadline)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                entries.remove(&id);
            }
        }
        entries.insert(
            key_id.into(),
            Entry {
                value: value.clone(),
                deadline,
            },
        );
        Ok(value)
    }

    /// source_ip must be the socket peer or verified trusted-proxy result.
    /// Explicit empty/wildcard/cross-app values never select a default tenant.
    /// Callers MUST independently check active tenant, feature and resource PDP
    /// on EVERY business request. No grant/hash/pepper leaves this method.
    pub async fn verify(
        &self,
        encoded: &str,
        pepper: &ApiKeyPepper,
        expected_app: Option<&str>,
        required_scope: &str,
        source_ip: IpAddr,
    ) -> Result<AuthorizedApiKey, CredentialAuthError> {
        let token =
            ApiKeyToken::parse(encoded).map_err(|_| CredentialAuthError::Unauthenticated)?;
        let value = self
            .local(token.key_id())
            .await?
            .ok_or(CredentialAuthError::Unauthenticated)?;
        // Wall-clock expiry is checked AFTER awaited IO, not at request arrival.
        value
            .grant
            .authorize(
                &token,
                pepper,
                expected_app,
                required_scope,
                source_ip,
                chrono::Utc::now().timestamp_millis(),
            )
            .map_err(|error| match error {
                ApiKeyPolicyError::Forbidden => CredentialAuthError::Forbidden,
                ApiKeyPolicyError::Unauthenticated => CredentialAuthError::Unauthenticated,
                _ => CredentialAuthError::Unavailable,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_key::ApiKeyKind;
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Reader {
        value: Mutex<Option<LocalCredential>>,
        calls: AtomicUsize,
        fail: AtomicBool,
        hang: AtomicBool,
    }
    #[async_trait]
    impl LocalCredentialReader for Reader {
        async fn lookup(&self, _: &str) -> Result<Option<LocalCredential>, CredentialReadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            if self.hang.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(CredentialReadError::Unavailable);
            }
            Ok(self.value.lock().unwrap().clone())
        }
    }
    fn fixture() -> (
        String,
        ApiKeyPepper,
        CredentialKeyRecord,
        Arc<Reader>,
        CachedApiKeyVerifier,
    ) {
        let pepper = ApiKeyPepper::from_base64url(&URL_SAFE_NO_PAD.encode([42; 32])).unwrap();
        let token = ApiKeyToken::generate(ApiKeyKind::Live).unwrap();
        let record = CredentialKeyRecord {
            key_id: token.key_id().into(),
            service_account_id: "sa-a".into(),
            mode: 1,
            hash: token.digest(&pepper),
            pepper_fingerprint: pepper.fingerprint(),
            scopes: vec!["endpoint.read".into()],
            expires_at_ms: 0,
            ip_allow: vec!["192.0.2.0/24".into()],
            revoked: false,
            service_account_enabled: true,
        };
        let value =
            LocalCredential::new(&record.key_id, "app-a".into(), 9, record.clone()).unwrap();
        let reader = Arc::new(Reader {
            value: Mutex::new(Some(value)),
            ..Default::default()
        });
        let cache = CachedApiKeyVerifier::new(reader.clone());
        (
            token.into_issuance_token().to_string(),
            pepper,
            record,
            reader,
            cache,
        )
    }
    async fn verify(
        cache: &CachedApiKeyVerifier,
        token: &str,
        pepper: &ApiKeyPepper,
    ) -> Result<AuthorizedApiKey, CredentialAuthError> {
        cache
            .verify(
                token,
                pepper,
                Some("app-a"),
                "endpoint.read",
                "192.0.2.1".parse().unwrap(),
            )
            .await
    }

    #[tokio::test(start_paused = true)]
    async fn every_cache_hit_rechecks_secret_kind_pepper_binding_scope_and_ip() {
        let (encoded, pepper, record, reader, cache) = fixture();
        assert_eq!(
            verify(&cache, &encoded, &pepper)
                .await
                .unwrap()
                .service_account_id,
            "sa-a"
        );
        for app in ["", "*", "app-b"] {
            assert_eq!(
                cache
                    .verify(
                        &encoded,
                        &pepper,
                        Some(app),
                        "endpoint.read",
                        "192.0.2.1".parse().unwrap()
                    )
                    .await,
                Err(CredentialAuthError::Forbidden)
            );
        }
        assert_eq!(
            cache
                .verify(
                    &encoded,
                    &pepper,
                    None,
                    "management.read",
                    "192.0.2.1".parse().unwrap()
                )
                .await,
            Err(CredentialAuthError::Forbidden)
        );
        assert_eq!(
            cache
                .verify(
                    &encoded,
                    &pepper,
                    None,
                    "endpoint.read",
                    "198.51.100.1".parse().unwrap()
                )
                .await,
            Err(CredentialAuthError::Forbidden)
        );
        let other = ApiKeyToken::generate(ApiKeyKind::Live)
            .unwrap()
            .into_issuance_token();
        let wrong = format!(
            "flr_live_{}_{}",
            record.key_id,
            other.splitn(4, '_').nth(3).unwrap()
        );
        assert_eq!(
            verify(&cache, &wrong, &pepper).await,
            Err(CredentialAuthError::Unauthenticated)
        );
        assert_eq!(
            verify(
                &cache,
                &encoded.replacen("flr_live", "flr_test", 1),
                &pepper
            )
            .await,
            Err(CredentialAuthError::Unauthenticated)
        );
        let wrong_pepper = ApiKeyPepper::from_base64url(&URL_SAFE_NO_PAD.encode([43; 32])).unwrap();
        assert_eq!(
            verify(&cache, &encoded, &wrong_pepper).await,
            Err(CredentialAuthError::Unavailable)
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn local_revoke_disable_and_empty_snapshot_stop_at_positive_ttl_without_stale_grace() {
        let (encoded, pepper, mut record, reader, cache) = fixture();
        assert!(verify(&cache, &encoded, &pepper).await.is_ok());
        record.revoked = true;
        *reader.value.lock().unwrap() =
            Some(LocalCredential::new(&record.key_id, "app-a".into(), 10, record.clone()).unwrap());
        tokio::time::advance(POSITIVE_TTL).await;
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unauthenticated)
        );
        record.revoked = false;
        record.service_account_enabled = false;
        *reader.value.lock().unwrap() =
            Some(LocalCredential::new(&record.key_id, "app-a".into(), 11, record.clone()).unwrap());
        tokio::time::advance(POSITIVE_TTL).await;
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unauthenticated)
        );
        *reader.value.lock().unwrap() = None;
        tokio::time::advance(POSITIVE_TTL).await;
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unauthenticated)
        );
        reader.fail.store(true, Ordering::SeqCst);
        tokio::time::advance(NEGATIVE_TTL).await;
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unavailable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn malformed_and_unknown_keys_do_not_bypass_or_unboundedly_query_storage() {
        let (encoded, pepper, _, reader, cache) = fixture();
        for value in ["", "Bearer jwt", "flr_live", "default", &"x".repeat(10000)] {
            assert_eq!(
                verify(&cache, value, &pepper).await,
                Err(CredentialAuthError::Unauthenticated)
            );
        }
        assert_eq!(reader.calls.load(Ordering::SeqCst), 0);
        let saved = reader.value.lock().unwrap().take();
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unauthenticated)
        );
        *reader.value.lock().unwrap() = saved;
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unauthenticated)
        );
        tokio::time::advance(NEGATIVE_TTL).await;
        assert!(verify(&cache, &encoded, &pepper).await.is_ok());
        reader.fail.store(true, Ordering::SeqCst);
        assert!(verify(&cache, &encoded, &pepper).await.is_ok());
        tokio::time::advance(POSITIVE_TTL).await;
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unavailable)
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn same_selector_is_single_flight_and_waiters_are_bounded() {
        let (encoded, pepper, _, reader, cache) = fixture();
        let cache = Arc::new(cache);
        let pepper = Arc::new(pepper);
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (cache, encoded, pepper) = (cache.clone(), encoded.clone(), pepper.clone());
            tasks.push(tokio::spawn(async move {
                verify(&cache, &encoded, &pepper).await
            }));
        }
        for task in tasks {
            assert!(task.await.unwrap().is_ok());
        }
        assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn blocked_reads_timeout_and_no_stale_entry_or_permit_survives() {
        let (encoded, pepper, _, reader, cache) = fixture();
        assert!(verify(&cache, &encoded, &pepper).await.is_ok());
        tokio::time::advance(POSITIVE_TTL).await;
        reader.hang.store(true, Ordering::SeqCst);
        let started = Instant::now();
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unavailable)
        );
        assert_eq!(Instant::now() - started, READ_TIMEOUT);
        assert!(cache.entries.lock().unwrap().is_empty());
        assert_eq!(cache.reads.available_permits(), MAX_READS);
        reader.hang.store(false, Ordering::SeqCst);
        assert!(verify(&cache, &encoded, &pepper).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_id_flood_is_capacity_bounded_and_wrong_storage_selector_fails_closed() {
        let (encoded, pepper, record, reader, cache) = fixture();
        *reader.value.lock().unwrap() = None;
        for n in 0..CAPACITY + 10 {
            let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
            let id: String = (0..12)
                .map(|shift| alphabet[(n >> (5 * shift.min(2))) & 31] as char)
                .collect();
            assert!(cache.local(&id).await.unwrap().is_none());
            assert!(cache.entries.lock().unwrap().len() <= CAPACITY);
        }
        assert_eq!(cache.entries.lock().unwrap().len(), CAPACITY);
        assert!(LocalCredential::new("OTHERKEY2345", "app-a".into(), 9, record.clone()).is_err());
        assert!(LocalCredential::new(&record.key_id, "app-a".into(), 0, record.clone()).is_err());
        let mut wrong =
            LocalCredential::new(&record.key_id, "app-a".into(), 10, record.clone()).unwrap();
        wrong.key_id = "OTHERKEY2345".into();
        *reader.value.lock().unwrap() = Some(wrong);
        assert_eq!(
            verify(&cache, &encoded, &pepper).await,
            Err(CredentialAuthError::Unavailable)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn expired_projection_is_rejected_on_each_hit_not_cached_as_an_identity() {
        let (encoded, pepper, mut record, reader, cache) = fixture();
        record.expires_at_ms = 1;
        *reader.value.lock().unwrap() =
            Some(LocalCredential::new(&record.key_id, "app-a".into(), 10, record.clone()).unwrap());
        for _ in 0..2 {
            assert_eq!(
                verify(&cache, &encoded, &pepper).await,
                Err(CredentialAuthError::Unauthenticated)
            );
        }
        assert_eq!(reader.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cold_read_bulkhead_rejects_excess_without_queuing_and_releases_after_timeout() {
        let (_, _, _, reader, cache) = fixture();
        reader.hang.store(true, Ordering::SeqCst);
        let cache = Arc::new(cache);
        let mut tasks = Vec::new();
        for suffix in b'A'..=b'P' {
            let cache = cache.clone();
            tasks.push(tokio::spawn(async move {
                cache.local(&format!("ABCDEFGHIJK{}", suffix as char)).await
            }));
        }
        for _ in 0..100 {
            if reader.calls.load(Ordering::SeqCst) == MAX_READS {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(reader.calls.load(Ordering::SeqCst), MAX_READS);
        assert_eq!(cache.reads.available_permits(), 0);
        let started = Instant::now();
        assert!(matches!(
            cache.local("ABCDEFGHIJKQ").await,
            Err(CredentialAuthError::Unavailable)
        ));
        assert_eq!(
            Instant::now(),
            started,
            "excess is refused without a wait queue"
        );
        assert_eq!(reader.calls.load(Ordering::SeqCst), MAX_READS);
        for task in tasks {
            assert!(matches!(
                task.await.unwrap(),
                Err(CredentialAuthError::Unavailable)
            ));
        }
        assert_eq!(cache.reads.available_permits(), MAX_READS);
        assert!(cache.entries.lock().unwrap().is_empty());
    }
}
