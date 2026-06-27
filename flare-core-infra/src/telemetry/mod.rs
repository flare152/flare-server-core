//! 进程级日志与 trace 订阅器初始化（`tracing-subscriber` + `tracing-log` + 可选 OTLP），与具体业务配置解耦。
//!
//! IM 等上层可把 TOML 中的日志字段映射为 [LoggingSubscriberOptions] 后调用 [init_fmt_subscriber]；
//! 需要端到端 trace 时调用 [init_tracing_subscriber] 并传入 [OtlpTracingOptions]。

use std::error::Error;

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

type TelemetryInitResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

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

/// OTLP trace exporter 配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtlpTracingOptions {
    pub service_name: String,
    pub endpoint: String,
}

impl OtlpTracingOptions {
    pub fn new(service_name: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            endpoint: endpoint.into(),
        }
    }

    fn enabled(&self) -> bool {
        !self.service_name.trim().is_empty() && !self.endpoint.trim().is_empty()
    }
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

/// 无 `RUST_LOG` 时：业务用 `level`，第三方库降噪。
///
/// 与 `flare-im-core/scripts/start_server.sh` 中默认 `RUST_LOG` 保持同类项一致，避免本地 `logs/` 被
/// `sqlx` / `tokio` / NATS / gRPC 栈在 `trace` 下刷到数 GB。
fn default_env_filter(app_level: &str) -> EnvFilter {
    let s = format!(
        "{},hyper=warn,reqwest=warn,h2=warn,rdkafka=warn,tower=warn,tokio=warn,sqlx=warn,tantivy=warn,async_nats=warn,tonic=warn,redis=warn",
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
    let _ = init_tracing_subscriber(options, None);
}

/// 安装全局 tracing subscriber，可选启用 OTLP trace exporter。
///
/// 已存在全局 subscriber 时静默跳过，保持重复初始化安全；OTLP exporter 构建失败会返回错误。
pub fn init_tracing_subscriber(
    options: Option<&LoggingSubscriberOptions>,
    otlp: Option<&OtlpTracingOptions>,
) -> TelemetryInitResult<()> {
    let _ = tracing_log::LogTracer::init();

    let opts = options.cloned().unwrap_or_default();
    let env_filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(_) => default_env_filter(opts.level.as_str()),
    };

    let use_ansi = opts.with_ansi.unwrap_or_else(stdout_is_terminal);
    if let Some(otlp) = otlp.filter(|options| options.enabled()) {
        init_subscriber_with_otlp(&opts, env_filter, use_ansi, otlp)?;
    } else {
        let fmt_layer = fmt::layer()
            .with_ansi(use_ansi)
            .with_target(opts.with_target)
            .with_thread_ids(opts.with_thread_ids)
            .with_file(opts.with_file)
            .with_line_number(opts.with_line_number);
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .try_init();
    }

    Ok(())
}

#[cfg(feature = "otel")]
fn init_subscriber_with_otlp(
    opts: &LoggingSubscriberOptions,
    env_filter: EnvFilter,
    use_ansi: bool,
    otlp: &OtlpTracingOptions,
) -> TelemetryInitResult<()> {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;
    use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otlp.endpoint.trim().to_string())
        .build()?;
    let provider = SdkTracerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name(otlp.service_name.trim().to_string())
                .build(),
        )
        .with_batch_exporter(exporter)
        .build();
    let tracer = provider.tracer(otlp.service_name.trim().to_string());
    opentelemetry::global::set_tracer_provider(provider);

    let fmt_layer = fmt::layer()
        .with_ansi(use_ansi)
        .with_target(opts.with_target)
        .with_thread_ids(opts.with_thread_ids)
        .with_file(opts.with_file)
        .with_line_number(opts.with_line_number);
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init();

    Ok(())
}

#[cfg(not(feature = "otel"))]
fn init_subscriber_with_otlp(
    opts: &LoggingSubscriberOptions,
    env_filter: EnvFilter,
    use_ansi: bool,
    otlp: &OtlpTracingOptions,
) -> TelemetryInitResult<()> {
    let fmt_layer = fmt::layer()
        .with_ansi(use_ansi)
        .with_target(opts.with_target)
        .with_thread_ids(opts.with_thread_ids)
        .with_file(opts.with_file)
        .with_line_number(opts.with_line_number);
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();
    tracing::warn!(
        service_name = %otlp.service_name,
        endpoint = %otlp.endpoint,
        "OTLP tracing requested but flare-core-infra was built without the `otel` feature"
    );
    Ok(())
}
