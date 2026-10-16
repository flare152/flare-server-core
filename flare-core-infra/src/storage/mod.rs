//! Business-neutral storage infrastructure, enabled with the `postgres` feature.

pub mod id_lease;
mod pg_acquire;

pub use pg_acquire::{PgAcquireMetrics, PgAcquireSnapshot};
