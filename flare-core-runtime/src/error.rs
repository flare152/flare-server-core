//! 统一错误类型定义
//!
//! 使用 thiserror 定义所有错误类型，确保错误信息清晰、可追踪

use std::time::Duration;
use thiserror::Error;

/// 运行时错误
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Task '{name}' failed: {source}")]
    TaskFailed {
        name: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Circular dependency detected: {tasks:?}")]
    CircularDependency { tasks: Vec<String> },

    #[error("Task '{task}' depends on '{dependency}', but '{dependency}' is not registered")]
    MissingDependency { task: String, dependency: String },

    #[error("Task '{name}' startup timeout after {timeout:?}")]
    StartupTimeout { name: String, timeout: Duration },

    #[error("Task '{name}' shutdown timeout after {timeout:?}")]
    ShutdownTimeout { name: String, timeout: Duration },

    #[error("Health check failed for '{name}': {source}")]
    HealthCheckFailed {
        name: String,
        #[source]
        source: HealthError,
    },

    #[error("Service registration failed: {0}")]
    RegistrationFailed(#[from] RegistryError),

    #[error("Plugin '{name}' initialization failed: {source}")]
    PluginInitFailed {
        name: String,
        #[source]
        source: PluginError,
    },

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Runtime is already running")]
    AlreadyRunning,

    #[error("Runtime is not running")]
    NotRunning,
}

/// 服务注册错误
#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Failed to connect to registry server: {0}")]
    ConnectionFailed(String),

    #[error("Failed to register service '{service_id}': {reason}")]
    RegistrationFailed { service_id: String, reason: String },

    #[error("Failed to deregister service '{service_id}': {reason}")]
    DeregistrationFailed { service_id: String, reason: String },

    #[error("Failed to send heartbeat for service '{service_id}': {reason}")]
    HeartbeatFailed { service_id: String, reason: String },

    #[error("Service '{service_id}' not found")]
    ServiceNotFound { service_id: String },

    #[error("Registry timeout after {timeout:?}")]
    Timeout { timeout: Duration },
}

/// 健康检查错误
#[derive(Debug, Error)]
pub enum HealthError {
    #[error("Health check '{name}' failed: {reason}")]
    CheckFailed { name: String, reason: String },

    #[error("Health check '{name}' timeout after {timeout:?}")]
    Timeout { name: String, timeout: Duration },

    #[error("Health check '{name}' is unhealthy: {details}")]
    Unhealthy { name: String, details: String },

    #[error("Consecutive failures exceeded threshold: {failures}/{threshold}")]
    ExceededThreshold { failures: u32, threshold: u32 },
}

/// 插件错误
#[derive(Debug, Error)]
pub enum PluginError {
    #[error("Plugin '{name}' initialization failed: {reason}")]
    InitFailed { name: String, reason: String },

    #[error("Plugin '{name}' execution failed: {reason}")]
    ExecutionFailed { name: String, reason: String },

    #[error("Plugin '{name}' not found")]
    NotFound { name: String },

    #[error("Plugin hook '{hook}' failed for plugin '{plugin}': {reason}")]
    HookFailed {
        plugin: String,
        hook: String,
        reason: String,
    },
}

/// 中间件错误
#[derive(Debug, Error)]
pub enum MiddlewareError {
    #[error("Middleware '{name}' execution failed: {reason}")]
    ExecutionFailed { name: String, reason: String },

    #[error("Middleware chain interrupted by '{name}': {reason}")]
    ChainInterrupted { name: String, reason: String },

    #[error("Middleware '{name}' not found")]
    NotFound { name: String },
}

/// 指标收集错误
#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Failed to collect metrics: {0}")]
    CollectionFailed(String),

    #[error("Failed to export metrics: {0}")]
    ExportFailed(String),

    #[error("Invalid metric name: {0}")]
    InvalidName(String),
}
