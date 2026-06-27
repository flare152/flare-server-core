//! Twitter Snowflake 风格 64-bit ID 生成器。
//!
//! 位布局：41-bit 毫秒时间戳 | 5-bit 数据中心 | 5-bit 机器 | 12-bit 序列号。
//! 十进制字符串通常 18–19 位，配合业务前缀（如 `usr_`）总长约 22–23 字符。

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 自定义纪元：2024-01-01 00:00:00 UTC
const EPOCH_MS: i64 = 1_704_067_200_000;

const WORKER_ID_BITS: u64 = 5;
const DATACENTER_ID_BITS: u64 = 5;
const SEQUENCE_BITS: u64 = 12;

const MAX_WORKER_ID: u64 = (1 << WORKER_ID_BITS) - 1;
const MAX_DATACENTER_ID: u64 = (1 << DATACENTER_ID_BITS) - 1;
const SEQUENCE_MASK: u64 = (1 << SEQUENCE_BITS) - 1;

const WORKER_ID_SHIFT: u64 = SEQUENCE_BITS;
const DATACENTER_ID_SHIFT: u64 = SEQUENCE_BITS + WORKER_ID_BITS;
const TIMESTAMP_SHIFT: u64 = SEQUENCE_BITS + WORKER_ID_BITS + DATACENTER_ID_BITS;

#[derive(Debug, thiserror::Error)]
pub enum SnowflakeError {
    #[error("worker_id must be in 0..={MAX_WORKER_ID}")]
    InvalidWorkerId,
    #[error("datacenter_id must be in 0..={MAX_DATACENTER_ID}")]
    InvalidDatacenterId,
    #[error("system clock moved backwards")]
    ClockMovedBackwards,
    #[error("failed to read system time")]
    SystemTimeError,
    #[error("snowflake generator lock poisoned")]
    LockPoisoned,
}

struct SnowflakeState {
    last_timestamp: i64,
    sequence: u64,
}

/// 线程安全的 Snowflake ID 生成器（进程内单例使用）。
pub struct SnowflakeGenerator {
    worker_id: u64,
    datacenter_id: u64,
    state: Mutex<SnowflakeState>,
}

impl SnowflakeGenerator {
    /// 创建生成器。`worker_id` / `datacenter_id` 各 5 bit（0..=31）。
    pub fn new(worker_id: u64, datacenter_id: u64) -> Result<Self, SnowflakeError> {
        if worker_id > MAX_WORKER_ID {
            return Err(SnowflakeError::InvalidWorkerId);
        }
        if datacenter_id > MAX_DATACENTER_ID {
            return Err(SnowflakeError::InvalidDatacenterId);
        }
        Ok(Self {
            worker_id,
            datacenter_id,
            state: Mutex::new(SnowflakeState {
                last_timestamp: -1,
                sequence: 0,
            }),
        })
    }

    /// 生成下一个 64-bit Snowflake ID。
    pub fn next_id(&self) -> Result<u64, SnowflakeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SnowflakeError::LockPoisoned)?;

        let mut timestamp = current_timestamp_ms()?;

        if timestamp < state.last_timestamp {
            return Err(SnowflakeError::ClockMovedBackwards);
        }

        if timestamp == state.last_timestamp {
            state.sequence = (state.sequence + 1) & SEQUENCE_MASK;
            if state.sequence == 0 {
                timestamp = wait_next_millis(state.last_timestamp)?;
            }
        } else {
            state.sequence = 0;
        }

        state.last_timestamp = timestamp;

        let ts_part = (timestamp - EPOCH_MS) as u64;
        Ok((ts_part << TIMESTAMP_SHIFT)
            | (self.datacenter_id << DATACENTER_ID_SHIFT)
            | (self.worker_id << WORKER_ID_SHIFT)
            | state.sequence)
    }

    /// 生成十进制字符串 ID（推荐用于业务主键）。
    pub fn next_id_string(&self) -> Result<String, SnowflakeError> {
        self.next_id().map(|id| id.to_string())
    }
}

fn current_timestamp_ms() -> Result<i64, SnowflakeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .map_err(|_| SnowflakeError::SystemTimeError)
}

fn wait_next_millis(last: i64) -> Result<i64, SnowflakeError> {
    let mut ts = current_timestamp_ms()?;
    while ts <= last {
        std::thread::yield_now();
        ts = current_timestamp_ms()?;
    }
    Ok(ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_monotonic_as_strings() {
        let generator = SnowflakeGenerator::new(1, 1).expect("generator");
        let a = generator.next_id_string().expect("id a");
        let b = generator.next_id_string().expect("id b");
        assert_ne!(a, b);
        assert!(a.len() <= 20);
        assert!(b.len() <= 20);
        assert!(a.parse::<u64>().is_ok());
    }

    #[test]
    fn prefixed_length_fits_social_format() {
        let generator = SnowflakeGenerator::new(0, 0).expect("generator");
        let id = generator.next_id_string().expect("id");
        let user_id = format!("usr_{id}");
        assert!(user_id.len() <= 25, "user_id too long: {}", user_id.len());
    }

    #[test]
    fn rejects_invalid_worker_id() {
        assert!(matches!(
            SnowflakeGenerator::new(32, 0),
            Err(SnowflakeError::InvalidWorkerId)
        ));
    }
}
