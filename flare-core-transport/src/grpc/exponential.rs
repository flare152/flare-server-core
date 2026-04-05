use super::RetryPolicy;
use std::time::Duration;
use tonic::Status;

/// 指数退避重试策略
pub struct ExponentialBackoffPolicy {
    max_attempts: usize,
    base_delay: Duration,
    max_delay: Duration,
}

impl ExponentialBackoffPolicy {
    pub fn new(max_attempts: usize, base_delay: Duration, max_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            max_delay,
        }
    }
}

impl RetryPolicy for ExponentialBackoffPolicy {
    fn should_retry(&self, attempt: usize, error: &Status) -> bool {
        if attempt >= self.max_attempts {
            return false;
        }

        // 只对可重试的错误进行重试
        matches!(
            error.code(),
            tonic::Code::Unavailable
                | tonic::Code::DeadlineExceeded
                | tonic::Code::ResourceExhausted
        )
    }

    fn backoff_duration(&self, attempt: usize) -> Duration {
        let delay_ms = self.base_delay.as_millis() as u64 * (1 << attempt.min(10));
        let delay = Duration::from_millis(delay_ms);
        delay.min(self.max_delay)
    }

    fn max_attempts(&self) -> usize {
        self.max_attempts
    }
}
