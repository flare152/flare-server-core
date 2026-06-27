//! 分布式 ID 生成

mod snowflake;

pub use snowflake::{SnowflakeError, SnowflakeGenerator};
