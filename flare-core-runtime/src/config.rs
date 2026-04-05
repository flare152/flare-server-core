//! 运行时配置模块
//!
//! 提供运行时配置，包括生命周期、任务启动、健康检查、指标等配置

use std::time::Duration;

/// 通用轮询型后台任务参数（与具体 Broker 无关）
///
/// 用于 MQ 消费者、定时轮询任务等
#[derive(Debug, Clone)]
pub struct PollWorkerConfig {
    /// 同时处理中的任务上限（如 Kafka 消费并发）
    pub concurrency: usize,
    /// `fetch` 无数据时的休眠间隔
    pub idle_backoff: Duration,
    /// `fetch` 失败后的退避间隔
    pub error_backoff: Duration,
}

impl Default for PollWorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            idle_backoff: Duration::from_millis(100),
            error_backoff: Duration::from_secs(1),
        }
    }
}

impl PollWorkerConfig {
    /// 创建默认配置
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置并发度
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// 设置空闲退避时间
    pub fn with_idle_backoff(mut self, d: Duration) -> Self {
        self.idle_backoff = d;
        self
    }

    /// 设置错误退避时间
    pub fn with_error_backoff(mut self, d: Duration) -> Self {
        self.error_backoff = d;
        self
    }
}

/// 任务启动配置
#[derive(Debug, Clone)]
pub struct TaskStartupConfig {
    /// 任务启动并发度（默认为 CPU 核心数）
    pub concurrency: usize,
    /// 任务启动超时时间（默认 30 秒）
    pub timeout: Duration,
    /// 是否启用任务就绪检查（默认 true）
    pub enable_ready_check: bool,
    /// 就绪检查超时时间（默认 30 秒）
    pub ready_check_timeout: Duration,
}

impl Default for TaskStartupConfig {
    fn default() -> Self {
        Self {
            concurrency: num_cpus::get(),
            timeout: Duration::from_secs(30),
            enable_ready_check: true,
            ready_check_timeout: Duration::from_secs(30),
        }
    }
}

impl TaskStartupConfig {
    /// 创建默认配置
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置启动并发度
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// 设置启动超时时间
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 启用/禁用就绪检查
    pub fn with_ready_check(mut self, enable: bool) -> Self {
        self.enable_ready_check = enable;
        self
    }

    /// 设置就绪检查超时时间
    pub fn with_ready_check_timeout(mut self, timeout: Duration) -> Self {
        self.ready_check_timeout = timeout;
        self
    }
}

/// 健康检查配置
#[derive(Debug, Clone)]
pub struct HealthCheckConfig {
    /// 健康检查间隔（默认 10 秒）
    pub interval: Duration,
    /// 健康检查超时时间（默认 5 秒）
    pub timeout: Duration,
    /// 连续失败阈值（默认 3 次）
    pub failure_threshold: u32,
    /// 是否启用健康检查（默认 true）
    pub enabled: bool,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(5),
            failure_threshold: 3,
            enabled: true,
        }
    }
}

impl HealthCheckConfig {
    /// 创建默认配置
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置检查间隔
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// 设置超时时间
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 设置失败阈值
    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }

    /// 启用/禁用健康检查
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// 指标配置
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    /// 是否启用指标收集（默认 true）
    pub enabled: bool,
    /// 指标暴露端口（默认 9090）
    pub port: u16,
    /// 指标暴露路径（默认 "/metrics"）
    pub path: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 9090,
            path: "/metrics".to_string(),
        }
    }
}

impl MetricsConfig {
    /// 创建默认配置
    pub fn new() -> Self {
        Self::default()
    }

    /// 启用/禁用指标收集
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 设置暴露端口
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// 设置暴露路径
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }
}

/// 微服务运行时配置
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// 关闭超时时间（默认 5 秒）
    pub shutdown_timeout: Duration,
    /// 任务启动配置
    pub task_startup: TaskStartupConfig,
    /// 健康检查配置
    pub health_check: HealthCheckConfig,
    /// 指标配置
    pub metrics: MetricsConfig,
    /// 默认轮询工作线程参数
    pub default_poll_worker: PollWorkerConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            shutdown_timeout: Duration::from_secs(5),
            task_startup: TaskStartupConfig::default(),
            health_check: HealthCheckConfig::default(),
            metrics: MetricsConfig::default(),
            default_poll_worker: PollWorkerConfig::default(),
        }
    }
}

impl RuntimeConfig {
    /// 创建默认配置
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置关闭超时时间
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// 设置任务启动配置
    pub fn with_task_startup(mut self, config: TaskStartupConfig) -> Self {
        self.task_startup = config;
        self
    }

    /// 设置健康检查配置
    pub fn with_health_check(mut self, config: HealthCheckConfig) -> Self {
        self.health_check = config;
        self
    }

    /// 设置指标配置
    pub fn with_metrics(mut self, config: MetricsConfig) -> Self {
        self.metrics = config;
        self
    }

    /// 设置默认轮询工作线程参数
    pub fn with_default_poll_worker(mut self, config: PollWorkerConfig) -> Self {
        self.default_poll_worker = config;
        self
    }

    /// 验证配置
    pub fn validate(&self) -> Result<(), String> {
        if self.shutdown_timeout.is_zero() {
            return Err("shutdown_timeout must be greater than zero".to_string());
        }

        if self.task_startup.concurrency == 0 {
            return Err("task_startup.concurrency must be greater than zero".to_string());
        }

        if self.health_check.failure_threshold == 0 {
            return Err("health_check.failure_threshold must be greater than zero".to_string());
        }

        Ok(())
    }
}
