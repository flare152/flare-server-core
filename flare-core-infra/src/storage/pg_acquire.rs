//! Bounded, label-free observation of SQLx PostgreSQL connection acquisition.
//!
//! Elapsed time covers the entire `PgPool::acquire()` future, including permit
//! waiting, dialing, validation and pool hooks. It is neither pure queue wait
//! nor SQL/query latency. No pool, URL, SQL, tenant, credential or error text is
//! retained. Observing acquisition does not change SQLx's timeout or policy.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sqlx::pool::PoolConnection;
use sqlx::{PgPool, Postgres};

/// A lock-free, read-only observation of one explicitly instrumented scope.
///
/// Fields are read independently, not as a transactional snapshot. `active`
/// counts in-progress acquisition futures, not leased connections. Terminal
/// counters balance `started` once observations quiesce. Monotonic counters
/// and elapsed aggregates saturate at `u64::MAX` instead of wrapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PgAcquireSnapshot {
    pub started: u64,
    pub active: u64,
    pub peak: u64,
    pub success: u64,
    pub timeout: u64,
    pub error: u64,
    pub cancelled: u64,
    pub elapsed_nanos: u64,
    pub elapsed_max_nanos: u64,
}

#[derive(Debug, Default)]
struct Counters {
    started: AtomicU64,
    active: AtomicU64,
    peak: AtomicU64,
    success: AtomicU64,
    timeout: AtomicU64,
    error: AtomicU64,
    cancelled: AtomicU64,
    elapsed_nanos: AtomicU64,
    elapsed_max_nanos: AtomicU64,
}

/// Clones share only fixed-size atomics. Creating or scraping this collector
/// does not acquire a connection, execute SQL or create a background task.
#[derive(Debug, Clone, Default)]
pub struct PgAcquireMetrics {
    counters: Arc<Counters>,
}

impl PgAcquireMetrics {
    pub async fn acquire(&self, pool: &PgPool) -> Result<PoolConnection<Postgres>, sqlx::Error> {
        self.observe(pool.acquire()).await
    }

    async fn observe<T>(
        &self,
        acquisition: impl Future<Output = Result<T, sqlx::Error>>,
    ) -> Result<T, sqlx::Error> {
        let observation = Observation::start(&self.counters, Instant::now());
        let result = acquisition.await;
        let terminal = match &result {
            Ok(_) => Terminal::Success,
            Err(sqlx::Error::PoolTimedOut) => Terminal::Timeout,
            Err(_) => Terminal::Error,
        };
        observation.finish(terminal, Instant::now());
        result
    }

    pub fn snapshot(&self) -> PgAcquireSnapshot {
        let c = &self.counters;
        PgAcquireSnapshot {
            started: c.started.load(Ordering::Relaxed),
            active: c.active.load(Ordering::Relaxed),
            peak: c.peak.load(Ordering::Relaxed),
            success: c.success.load(Ordering::Relaxed),
            timeout: c.timeout.load(Ordering::Relaxed),
            error: c.error.load(Ordering::Relaxed),
            cancelled: c.cancelled.load(Ordering::Relaxed),
            elapsed_nanos: c.elapsed_nanos.load(Ordering::Relaxed),
            elapsed_max_nanos: c.elapsed_max_nanos.load(Ordering::Relaxed),
        }
    }
}

enum Terminal {
    Success,
    Timeout,
    Error,
    Cancelled,
}

struct Observation<'a> {
    counters: &'a Counters,
    started: Instant,
    finished: bool,
}

fn saturating_add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

impl<'a> Observation<'a> {
    fn start(counters: &'a Counters, started: Instant) -> Self {
        saturating_add(&counters.started, 1);
        let active = counters.active.fetch_add(1, Ordering::Relaxed) + 1;
        counters.peak.fetch_max(active, Ordering::Relaxed);
        Self {
            counters,
            started,
            finished: false,
        }
    }

    fn finish(mut self, terminal: Terminal, now: Instant) {
        self.record(terminal, now.saturating_duration_since(self.started));
    }

    fn record(&mut self, terminal: Terminal, elapsed: Duration) {
        self.finished = true;
        let c = self.counters;
        let counter = match terminal {
            Terminal::Success => &c.success,
            Terminal::Timeout => &c.timeout,
            Terminal::Error => &c.error,
            Terminal::Cancelled => &c.cancelled,
        };
        saturating_add(counter, 1);
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        saturating_add(&c.elapsed_nanos, nanos);
        c.elapsed_max_nanos.fetch_max(nanos, Ordering::Relaxed);
        c.active.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Drop for Observation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.record(Terminal::Cancelled, self.started.elapsed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{pending, ready};
    use std::task::{Context, Poll, Waker};

    #[test]
    fn pg_acq_01_terminals_once_and_deterministic_elapsed() {
        let metrics = PgAcquireMetrics::default();
        let start = Instant::now();
        for (terminal, nanos) in [
            (Terminal::Success, 7),
            (Terminal::Timeout, 11),
            (Terminal::Error, 13),
            (Terminal::Cancelled, 17),
        ] {
            Observation::start(&metrics.counters, start)
                .finish(terminal, start + Duration::from_nanos(nanos));
        }
        assert_eq!(
            metrics.snapshot(),
            PgAcquireSnapshot {
                started: 4,
                active: 0,
                peak: 1,
                success: 1,
                timeout: 1,
                error: 1,
                cancelled: 1,
                elapsed_nanos: 48,
                elapsed_max_nanos: 17,
            }
        );
    }

    #[test]
    fn pg_acq_02_unpolled_and_cancelled_future() {
        let metrics = PgAcquireMetrics::default();
        drop(metrics.observe(pending::<Result<(), sqlx::Error>>()));
        assert_eq!(metrics.snapshot(), PgAcquireSnapshot::default());
        let mut acquisition = Box::pin(metrics.observe(pending::<Result<(), sqlx::Error>>()));
        assert!(matches!(
            acquisition
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert_eq!(metrics.snapshot().active, 1);
        drop(acquisition);
        let snapshot = metrics.snapshot();
        assert_eq!(
            (snapshot.started, snapshot.active, snapshot.cancelled),
            (1, 0, 1)
        );
        assert_eq!(
            (snapshot.success, snapshot.timeout, snapshot.error),
            (0, 0, 0)
        );
    }

    #[tokio::test]
    async fn pg_acq_03_passthrough_result_and_error_classification() {
        let metrics = PgAcquireMetrics::default();
        assert_eq!(
            metrics
                .observe(ready(Ok::<_, sqlx::Error>(42)))
                .await
                .unwrap(),
            42
        );
        for error in [
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            sqlx::Error::RowNotFound,
        ] {
            let result = metrics.observe(ready(Err::<(), _>(error))).await;
            assert!(result.is_err());
        }
        let snapshot = metrics.snapshot();
        assert_eq!(
            (
                snapshot.started,
                snapshot.success,
                snapshot.timeout,
                snapshot.error,
                snapshot.cancelled
            ),
            (4, 1, 1, 2, 0)
        );
        assert_eq!(snapshot.active, 0);
    }

    #[test]
    fn pg_acq_04_shared_clone_peak_and_read_only_scrapes() {
        let metrics = PgAcquireMetrics::default();
        let clone = metrics.clone();
        let first = Observation::start(&metrics.counters, Instant::now());
        let second = Observation::start(&clone.counters, Instant::now());
        let before = metrics.snapshot();
        assert_eq!((before.active, before.peak), (2, 2));
        for _ in 0..64 {
            assert_eq!(clone.snapshot(), before);
        }
        first.finish(Terminal::Success, Instant::now());
        drop(second);
        let after = metrics.snapshot();
        assert_eq!(
            (
                after.started,
                after.active,
                after.peak,
                after.success,
                after.cancelled
            ),
            (2, 0, 2, 1, 1)
        );
    }

    #[test]
    fn pg_acq_05_elapsed_aggregates_saturate_without_wrapping() {
        let metrics = PgAcquireMetrics::default();
        metrics
            .counters
            .elapsed_nanos
            .store(u64::MAX - 2, Ordering::Relaxed);
        let start = Instant::now();
        Observation::start(&metrics.counters, start)
            .finish(Terminal::Success, start + Duration::from_nanos(10));
        assert_eq!(metrics.snapshot().elapsed_nanos, u64::MAX);
        assert_eq!(metrics.snapshot().elapsed_max_nanos, 10);
    }

    #[tokio::test]
    async fn pg_acq_06_closed_lazy_pool_no_network() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy_with(sqlx::postgres::PgConnectOptions::new());
        pool.close().await;
        let metrics = PgAcquireMetrics::default();
        assert!(matches!(
            metrics.acquire(&pool).await,
            Err(sqlx::Error::PoolClosed)
        ));
        let snapshot = metrics.snapshot();
        assert_eq!(
            (snapshot.started, snapshot.active, snapshot.error),
            (1, 0, 1)
        );
    }

    /// Explicit local-PostgreSQL component test, never part of default tests.
    /// Requires FLARE_PG_ACQUIRE_TEST_DATABASE_URL; performs SELECT only, forces
    /// read-only sessions, owns its pool, and has a 10 s total test deadline.
    /// It neither creates/drops databases nor proves production gateway/SLOs.
    #[tokio::test]
    #[ignore = "requires explicitly provided local PostgreSQL; SELECT-only component test"]
    async fn pg_acq_07_real_pg_timeout_cancel_recovery_and_closed() {
        use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
        use std::str::FromStr;

        let url = std::env::var("FLARE_PG_ACQUIRE_TEST_DATABASE_URL")
            .unwrap_or_else(|_| panic!("local PostgreSQL test URL must be explicitly supplied"));
        let options = PgConnectOptions::from_str(&url)
            .unwrap_or_else(|_| panic!("invalid local PostgreSQL test configuration"));
        assert!(
            matches!(options.get_host(), "127.0.0.1" | "::1" | "localhost"),
            "PostgreSQL test requires a loopback host"
        );
        let options = options
            .application_name("flare-pg-acquire-component-test")
            .options([
                ("default_transaction_read_only", "on"),
                ("statement_timeout", "1000"),
            ]);
        let run = async {
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_millis(750))
                .connect_lazy_with(options);
            let metrics = PgAcquireMetrics::default();
            let mut held = metrics
                .acquire(&pool)
                .await
                .unwrap_or_else(|_| panic!("local PostgreSQL initial acquisition failed"));
            let read_only: bool =
                sqlx::query_scalar("SELECT current_setting('transaction_read_only') = 'on'")
                    .fetch_one(&mut *held)
                    .await
                    .unwrap_or_else(|_| panic!("local PostgreSQL read-only probe failed"));
            assert!(read_only);
            assert_eq!(
                (metrics.snapshot().active, metrics.snapshot().success),
                (0, 1)
            );

            assert!(matches!(
                metrics.acquire(&pool).await,
                Err(sqlx::Error::PoolTimedOut)
            ));
            let mut cancelled = Box::pin(metrics.acquire(&pool));
            // Poll once while the sole connection is held, then explicitly drop
            // the actual SQLx future. No sleep or detached task is involved.
            assert!(matches!(
                cancelled
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            assert_eq!(metrics.snapshot().active, 1);
            drop(cancelled);
            assert_eq!(metrics.snapshot().cancelled, 1);

            drop(held);
            let mut recovered = metrics
                .acquire(&pool)
                .await
                .unwrap_or_else(|_| panic!("local PostgreSQL acquisition did not recover"));
            let one: i32 = sqlx::query_scalar("SELECT 1")
                .fetch_one(&mut *recovered)
                .await
                .unwrap_or_else(|_| panic!("local PostgreSQL recovery SELECT failed"));
            assert_eq!(one, 1);
            drop(recovered);
            pool.close().await;
            assert!(matches!(
                metrics.acquire(&pool).await,
                Err(sqlx::Error::PoolClosed)
            ));
            let snapshot = metrics.snapshot();
            assert_eq!(
                (
                    snapshot.started,
                    snapshot.active,
                    snapshot.success,
                    snapshot.timeout,
                    snapshot.error,
                    snapshot.cancelled
                ),
                (5, 0, 2, 1, 1, 1)
            );
            assert!(snapshot.elapsed_nanos >= snapshot.elapsed_max_nanos);
            assert!(snapshot.elapsed_max_nanos >= Duration::from_millis(500).as_nanos() as u64);
        };
        assert!(
            tokio::time::timeout(Duration::from_secs(10), run)
                .await
                .is_ok(),
            "local PostgreSQL component test exceeded its total deadline"
        );
    }
}
