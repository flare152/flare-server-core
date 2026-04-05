//! 进程级日志订阅器初始化（`tracing-subscriber` + `tracing-log`），与具体业务配置解耦。
//!
//! IM 等上层可把 TOML 中的日志字段映射为 [LoggingSubscriberOptions] 后调用 [init_fmt_subscriber]；
//! OTLP / Tempo 等与业务 feature 矩阵强相关的逻辑可留在各自 crate（版本与依赖常不一致）。

use tracing_subscriber::{EnvFilter, fmt};

/// 与典型 YAML/TOML 日志段对齐的 fmt 层选项（不依赖任何应用配置类型）
#[derive(Debug, Clone)]
pub struct LoggingSubscriberOptions {
    pub level: String,
    pub with_target: bool,
    pub with_thread_ids: bool,
    pub with_file: bool,
    pub with_line_number: bool,
    pub with_ansi: Option<bool>,
}

impl Default for LoggingSubscriberOptions {
    fn default() -> Self {
        Self {
            level: "debug".to_string(),
            with_target: false,
            with_thread_ids: true,
            with_file: true,
            with_line_number: true,
            with_ansi: None,
        }
    }
}

/// 无 `RUST_LOG` 时：业务用 `level`，第三方库降噪
fn default_env_filter(app_level: &str) -> EnvFilter {
    let s = format!(
        "{},hyper=warn,reqwest=warn,h2=warn,rdkafka=warn,tower=warn,tokio=warn,sqlx=warn,tantivy=warn",
        app_level
    );
    EnvFilter::try_new(&s).unwrap_or_else(|_| EnvFilter::new(app_level))
}

fn stdout_is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

/// 安装全局 fmt subscriber：`log` crate 事件经 `tracing-log` 汇入同一流。
///
/// * `options` — `None` 时使用 [LoggingSubscriberOptions::default]。
/// * 已存在全局 subscriber 时静默跳过（`try_init`），避免测试/重复启动 panic。
pub fn init_fmt_subscriber(options: Option<&LoggingSubscriberOptions>) {
    let _ = tracing_log::LogTracer::init();

    let opts = options.cloned().unwrap_or_default();

    let env_filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => default_env_filter(opts.level.as_str()),
    };

    let use_ansi = opts.with_ansi.unwrap_or_else(stdout_is_terminal);

    let builder = fmt::Subscriber::builder()
        .with_ansi(use_ansi)
        .with_target(opts.with_target)
        .with_thread_ids(opts.with_thread_ids)
        .with_file(opts.with_file)
        .with_line_number(opts.with_line_number)
        .with_env_filter(env_filter);

    let _ = builder.try_init();
}
