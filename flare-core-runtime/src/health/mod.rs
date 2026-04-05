//! 健康检查模块
//!
//! 提供健康检查抽象和实现

mod checker;
mod r#trait;

pub use checker::{HealthCheckResult, HealthChecker};
pub use r#trait::HealthCheck;
