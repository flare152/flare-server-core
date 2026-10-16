//! Durable, bounded timestamp-range leases for Snowflake's 10-bit node identity.
//! Business owners choose a table and a disjoint slot range; no business SQL here.
//! A successor starts strictly above its predecessor's persisted timestamp fence.
use sqlx::PgPool;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const EPOCH_MS: i64 = 1_704_067_200_000;
const RESERVATION_MS: i64 = 30_000;

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum IdLeaseError {
    #[error("ID lease unavailable")]
    Unavailable,
    #[error("ID lease not active for this timestamp")]
    Inactive,
    #[error("ID clock moved backwards or exceeds supported epoch")]
    Clock,
    #[error("ID sequence exhausted for this millisecond")]
    Exhausted,
    #[error("invalid ID lease configuration")]
    Configuration,
}

pub struct LeasedSnowflake {
    slot: u16,
    first: i64,
    until: AtomicI64,
    enabled: AtomicBool,
    retired: AtomicBool,
    deadline: Mutex<Instant>,
    state: Mutex<(i64, u16)>,
}
impl LeasedSnowflake {
    pub fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }
    pub fn next_id_string(&self) -> Result<String, IdLeaseError> {
        self.next_at(now_ms()?).map(|value| value.to_string())
    }
    fn next_at(&self, now: i64) -> Result<u64, IdLeaseError> {
        let mut state = self.state.lock().map_err(|_| IdLeaseError::Unavailable)?;
        if Instant::now()
            > *self
                .deadline
                .lock()
                .map_err(|_| IdLeaseError::Unavailable)?
            || self.retired.load(Ordering::Acquire)
            || !self.enabled.load(Ordering::Acquire)
            || now < self.first
            || now > self.until.load(Ordering::Acquire)
        {
            return Err(IdLeaseError::Inactive);
        }
        if now < state.0 || !(EPOCH_MS..EPOCH_MS + (1_i64 << 41)).contains(&now) {
            return Err(IdLeaseError::Clock);
        }
        let sequence = if now == state.0 {
            state
                .1
                .checked_add(1)
                .filter(|v| *v < 4096)
                .ok_or(IdLeaseError::Exhausted)?
        } else {
            0
        };
        *state = (now, sequence);
        Ok((((now - EPOCH_MS) as u64) << 22) | (u64::from(self.slot) << 12) | u64::from(sequence))
    }
}

pub struct IdLeaseGuard {
    generator: Arc<LeasedSnowflake>,
    task: tokio::task::JoinHandle<()>,
}
impl IdLeaseGuard {
    pub fn generator(&self) -> Arc<LeasedSnowflake> {
        self.generator.clone()
    }
}
impl Drop for IdLeaseGuard {
    fn drop(&mut self) {
        self.generator.retired.store(true, Ordering::Release);
        self.generator.enabled.store(false, Ordering::Release);
        self.task.abort();
        // Do not release early: persisted timestamp fences survive process exit.
    }
}

fn now_ms() -> Result<i64, IdLeaseError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
        .ok_or(IdLeaseError::Clock)
}
fn table_name(table: &str) -> Result<String, IdLeaseError> {
    let parts: Vec<_> = table.split('.').collect();
    if parts.len() != 2
        || parts.iter().any(|v| {
            v.is_empty()
                || v.len() > 63
                || !v.as_bytes()[0].is_ascii_lowercase()
                || !v
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        })
    {
        return Err(IdLeaseError::Configuration);
    }
    Ok(format!("\"{}\".\"{}\"", parts[0], parts[1]))
}

/// Uses the supplied owning pool (no extra pool or DDL). SQL must have seeded
/// slots in the specified range. A lost/failed renewal closes local generation;
/// later success can resume only the same owner, never silently a new identity.
pub async fn acquire_id_lease(
    pool: PgPool,
    table: &str,
    first_slot: u16,
    last_slot: u16,
) -> Result<IdLeaseGuard, IdLeaseError> {
    if first_slot > last_slot || last_slot > 1023 {
        return Err(IdLeaseError::Configuration);
    }
    let table = table_name(table)?;
    let owner = uuid::Uuid::new_v4();
    let started = Instant::now();
    let sql = format!(
        "WITH clock AS MATERIALIZED (SELECT floor(extract(epoch FROM clock_timestamp())*1000)::bigint AS ms), candidate AS (SELECT t.slot,t.until_ms FROM {table} t CROSS JOIN clock WHERE t.slot BETWEEN $1 AND $2 AND t.until_ms<clock.ms ORDER BY t.until_ms,t.slot FOR UPDATE OF t SKIP LOCKED LIMIT 1) UPDATE {table} t SET owner=$3::uuid,until_ms=GREATEST(clock.ms,c.until_ms+1)+$4 FROM candidate c,clock WHERE t.slot=c.slot RETURNING t.slot,GREATEST(clock.ms,c.until_ms+1),t.until_ms"
    );
    let (slot, first, until): (i16, i64, i64) = tokio::time::timeout(
        Duration::from_secs(3),
        sqlx::query_as(&sql)
            .bind(first_slot as i16)
            .bind(last_slot as i16)
            .bind(owner.to_string())
            .bind(RESERVATION_MS)
            .fetch_optional(&pool),
    )
    .await
    .map_err(|_| IdLeaseError::Unavailable)?
    .map_err(|_| IdLeaseError::Unavailable)?
    .ok_or(IdLeaseError::Unavailable)?;
    let now = now_ms()?;
    // A materially skewed local clock cannot start issuing under a DB fence.
    if now < first || now > until {
        return Err(IdLeaseError::Clock);
    }
    let generator = Arc::new(LeasedSnowflake {
        slot: slot as u16,
        first,
        until: AtomicI64::new(until),
        enabled: AtomicBool::new(true),
        retired: AtomicBool::new(false),
        deadline: Mutex::new(started + Duration::from_millis(RESERVATION_MS as u64)),
        state: Mutex::new((-1, 0)),
    });
    let state = generator.clone();
    let task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            let was_enabled = state.enabled.load(Ordering::Acquire);
            if renew(&pool, &table, owner, &state).await.is_err() && was_enabled {
                tracing::warn!("ID lease renewal unavailable; local generation disabled");
            }
        }
    });
    Ok(IdLeaseGuard { generator, task })
}

async fn renew(
    pool: &PgPool,
    table: &str,
    owner: uuid::Uuid,
    state: &LeasedSnowflake,
) -> Result<(), IdLeaseError> {
    let result = renew_inner(pool, table, owner, state).await;
    if result.is_err() {
        state.enabled.store(false, Ordering::Release);
    }
    result
}

async fn renew_inner(
    pool: &PgPool,
    table: &str,
    owner: uuid::Uuid,
    state: &LeasedSnowflake,
) -> Result<(), IdLeaseError> {
    let started = Instant::now();
    let sql = format!(
        "UPDATE {table} SET until_ms=GREATEST(until_ms,floor(extract(epoch FROM clock_timestamp())*1000)::bigint+$3) WHERE slot=$1 AND owner=$2::uuid RETURNING until_ms"
    );
    let until: i64 = tokio::time::timeout(
        Duration::from_secs(3),
        sqlx::query_scalar(&sql)
            .bind(state.slot as i16)
            .bind(owner.to_string())
            .bind(RESERVATION_MS)
            .fetch_optional(pool),
    )
    .await
    .map_err(|_| IdLeaseError::Unavailable)?
    .map_err(|_| IdLeaseError::Unavailable)?
    .ok_or(IdLeaseError::Inactive)?;
    state.until.store(until, Ordering::Release);
    *state
        .deadline
        .lock()
        .map_err(|_| IdLeaseError::Unavailable)? =
        started + Duration::from_millis(RESERVATION_MS as u64);
    state.enabled.store(true, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn generator(slot: u16, first: i64, until: i64) -> LeasedSnowflake {
        LeasedSnowflake {
            slot,
            first,
            until: AtomicI64::new(until),
            enabled: AtomicBool::new(true),
            retired: AtomicBool::new(false),
            deadline: Mutex::new(Instant::now() + Duration::from_secs(30)),
            state: Mutex::new((-1, 0)),
        }
    }
    #[test]
    fn successor_ranges_and_node_partitions_never_overlap() {
        let t = EPOCH_MS + 60_000;
        let old = generator(0, t, t + 30_000);
        let next = generator(0, t + 30_001, t + 60_001);
        assert!(old.next_at(t + 30_000).unwrap() < next.next_at(t + 30_001).unwrap());
        assert_eq!(old.next_at(t + 30_001), Err(IdLeaseError::Inactive));
        assert_eq!(next.next_at(t + 30_000), Err(IdLeaseError::Inactive));
        assert_ne!(
            generator(511, t, t).next_at(t).unwrap(),
            generator(512, t, t).next_at(t).unwrap()
        );
    }
    #[test]
    fn clock_rollback_expiry_and_sequence_fail_closed_without_spin() {
        let t = EPOCH_MS + 60_000;
        let g = generator(0, t, t + 1);
        for _ in 0..4096 {
            g.next_at(t).unwrap();
        }
        assert_eq!(g.next_at(t), Err(IdLeaseError::Exhausted));
        g.next_at(t + 1).unwrap();
        assert_eq!(g.next_at(t), Err(IdLeaseError::Clock));
        assert_eq!(g.next_at(t + 2), Err(IdLeaseError::Inactive));
        g.enabled.store(false, Ordering::Release);
        assert_eq!(g.next_at(t + 1), Err(IdLeaseError::Inactive));
        g.enabled.store(true, Ordering::Release);
        *g.deadline.lock().unwrap() = Instant::now() - Duration::from_secs(1);
        assert_eq!(g.next_at(t + 1), Err(IdLeaseError::Inactive));
    }
    #[test]
    fn only_explicit_qualified_identifiers_are_accepted() {
        assert!(table_name("public.id_leases").is_ok());
        for bad in [
            "id_leases",
            "public.a;drop table x",
            "public.x.y",
            "public.\"x\"",
            "public._x",
        ] {
            assert!(table_name(bad).is_err());
        }
    }

    #[tokio::test]
    #[ignore = "requires explicitly owned private PG ID lease fixture"]
    async fn actual_pg_excludes_reuse_and_lost_owner_stops_generation() -> anyhow::Result<()> {
        use std::str::FromStr;
        let options = sqlx::postgres::PgConnectOptions::from_str(&std::env::var(
            "FLARE_ID_LEASE_TEST_PG_URL",
        )?)?;
        anyhow::ensure!(
            options.get_database() == Some("id_lease_test")
                && options
                    .get_socket()
                    .and_then(|v| v.to_str())
                    .is_some_and(|s| s.starts_with("/private/tmp/flare-id-lease.")),
            "private fixture required"
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with(options)
            .await?;
        let marker:Option<String>=sqlx::query_scalar("SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()").fetch_one(&pool).await?;
        anyhow::ensure!(
            marker.as_deref() == Some("flare-owned-id-lease-functional"),
            "fixture ownership required"
        );
        sqlx::raw_sql("CREATE TABLE public.test_id_leases(slot smallint PRIMARY KEY,owner uuid,until_ms bigint NOT NULL DEFAULT 0); INSERT INTO public.test_id_leases(slot) VALUES(0),(1);").execute(&pool).await?;
        let first = acquire_id_lease(pool.clone(), "public.test_id_leases", 0, 0).await?;
        let ids = first.generator();
        assert!(ids.next_id_string().is_ok());
        assert!(
            acquire_id_lease(pool.clone(), "public.test_id_leases", 0, 0)
                .await
                .is_err()
        );
        let other = acquire_id_lease(pool.clone(), "public.test_id_leases", 1, 1).await?;
        assert_ne!(ids.next_id_string()?, other.generator().next_id_string()?);
        let owner: String =
            sqlx::query_scalar("SELECT owner::text FROM public.test_id_leases WHERE slot=0")
                .fetch_one(&pool)
                .await?;
        let owner = uuid::Uuid::parse_str(&owner)?;
        renew(&pool, "public.test_id_leases", owner, &ids).await?;
        sqlx::query("UPDATE public.test_id_leases SET owner=$1::uuid WHERE slot=0")
            .bind(uuid::Uuid::new_v4().to_string())
            .execute(&pool)
            .await?;
        assert_eq!(
            renew(&pool, "public.test_id_leases", owner, &ids).await,
            Err(IdLeaseError::Inactive)
        );
        assert!(ids.next_id_string().is_err());
        let remaining = other.generator();
        drop(other);
        assert!(remaining.next_id_string().is_err());
        pool.close().await;
        assert!(
            renew(&pool, "public.test_id_leases", owner, &ids)
                .await
                .is_err()
        );
        assert!(ids.next_id_string().is_err());
        drop(first);
        Ok(())
    }
}
