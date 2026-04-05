//! 重试策略模块

pub mod exponential;
pub mod fixed;

pub use exponential::ExponentialBackoffPolicy;
pub use fixed::FixedRetryPolicy;

use std::time::Duration;

/// 重试策略 trait
pub trait RetryPolicy {
    fn should_retry(&self, attempt: usize, error: &tonic::Status) -> bool;
    fn backoff_duration(&self, attempt: usize) -> Duration;
    fn max_attempts(&self) -> usize;
}
