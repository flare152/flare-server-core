//! Business-neutral storage infrastructure, enabled with the `postgres` feature.

mod pg_acquire;

pub use pg_acquire::{PgAcquireMetrics, PgAcquireSnapshot};
