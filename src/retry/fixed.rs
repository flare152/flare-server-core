use super::RetryPolicy;
use std::time::Duration;
use tonic::Status;

/// 固定延迟重试策略
pub struct FixedRetryPolicy {
    max_attempts: usize,
    delay: Duration,
}

impl FixedRetryPolicy {
    pub fn new(max_attempts: usize, delay: Duration) -> Self {
        Self {
            max_attempts,
            delay,
        }
    }
}

impl RetryPolicy for FixedRetryPolicy {
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

    fn backoff_duration(&self, _attempt: usize) -> Duration {
        self.delay
    }

    fn max_attempts(&self) -> usize {
        self.max_attempts
    }
}
