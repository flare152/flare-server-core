//! 进程级日志与 trace 订阅器初始化（`tracing-subscriber` + `tracing-log` + 可选 OTLP），与具体业务配置解耦。
//!
//! IM 等上层可把 TOML 中的日志字段映射为 [LoggingSubscriberOptions] 后调用 [init_fmt_subscriber]；
//! 需要端到端 trace 时调用 [init_tracing_subscriber] 并传入 [OtlpTracingOptions]。

use std::error::Error;
use std::fmt as std_fmt;
use std::str::FromStr;
use std::time::Duration;

use tracing_subscriber::{
    EnvFilter, Layer, fmt, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt,
};

type TelemetryInitResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// stdout 日志行格式。
///
/// `Json` 每个事件一行 JSON（字段平铺、带当前 span），供 Vector/Promtail 等采集器直接解析；
/// 与 OTLP trace exporter 可同时启用，二者是同一个 subscriber 上的两个 layer。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

impl LogFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
        }
    }
}

impl std_fmt::Display for LogFormat {
    fn fmt(&self, f: &mut std_fmt::Formatter<'_>) -> std_fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 未知格式名。显式配置写错时应拒绝启动，而不是悄悄退回文本格式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownLogFormat(pub String);

impl std_fmt::Display for UnknownLogFormat {
    fn fmt(&self, f: &mut std_fmt::Formatter<'_>) -> std_fmt::Result {
        write!(
            f,
            "unknown log format `{}` (expected `text` or `json`)",
            self.0
        )
    }
}

impl Error for UnknownLogFormat {}

impl FromStr for LogFormat {
    type Err = UnknownLogFormat;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(UnknownLogFormat(value.to_string())),
        }
    }
}

/// 与典型 YAML/TOML 日志段对齐的 fmt 层选项（不依赖任何应用配置类型）
#[derive(Debug, Clone)]
pub struct LoggingSubscriberOptions {
    pub level: String,
    pub with_target: bool,
    pub with_thread_ids: bool,
    pub with_file: bool,
    pub with_line_number: bool,
    pub with_ansi: Option<bool>,
    pub format: LogFormat,
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
            format: LogFormat::Text,
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

/// 按选项构造 stdout fmt 层。JSON 永不带 ANSI 转义，否则采集端拿到的是被颜色码污染的字段。
fn fmt_layer<S>(opts: &LoggingSubscriberOptions, use_ansi: bool) -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    let layer = fmt::layer()
        .with_target(opts.with_target)
        .with_thread_ids(opts.with_thread_ids)
        .with_file(opts.with_file)
        .with_line_number(opts.with_line_number);
    match opts.format {
        LogFormat::Text => layer.with_ansi(use_ansi).boxed(),
        LogFormat::Json => layer
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_ansi(false)
            .boxed(),
    }
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
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer(&opts, use_ansi))
            .try_init();
    }

    Ok(())
}

/// 进程内唯一的 OTLP provider；[shutdown_tracing] 用它在退出前导出最后一批 span。
#[cfg(feature = "otel")]
static TRACER_PROVIDER: std::sync::OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> =
    std::sync::OnceLock::new();

/// 导出尚在批处理队列中的 span 并停止 OTLP exporter；未启用 OTLP 时直接返回。
///
/// 批处理器默认每 5 秒导出一次，进程退出前不调用会丢掉最后一批 span。
/// 必须在 tokio 运行时仍存活时调用（exporter 的 gRPC 通道依赖它）。
pub fn shutdown_tracing(timeout: Duration) -> TelemetryInitResult<()> {
    #[cfg(feature = "otel")]
    if let Some(provider) = TRACER_PROVIDER.get() {
        provider.shutdown_with_timeout(timeout)?;
    }
    #[cfg(not(feature = "otel"))]
    let _ = timeout;
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
    let _ = TRACER_PROVIDER.set(provider.clone());
    opentelemetry::global::set_tracer_provider(provider);

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer(opts, use_ansi))
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
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer(opts, use_ansi))
        .try_init();
    tracing::warn!(
        service_name = %otlp.service_name,
        endpoint = %otlp.endpoint,
        "OTLP tracing requested but flare-core-infra was built without the `otel` feature"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_parses_known_names_case_insensitively() {
        assert_eq!("text".parse::<LogFormat>(), Ok(LogFormat::Text));
        assert_eq!(" JSON ".parse::<LogFormat>(), Ok(LogFormat::Json));
        assert_eq!(LogFormat::default(), LogFormat::Text);
    }

    #[test]
    fn unknown_log_format_is_rejected_not_defaulted() {
        let error = "logfmt".parse::<LogFormat>().expect_err("must reject");
        assert!(error.to_string().contains("logfmt"));
        assert!("".parse::<LogFormat>().is_err());
    }

    #[test]
    fn json_layer_writes_one_parseable_object_per_event() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl Write for Buffer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("buffer lock").extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buffer = Buffer::default();
        let writer = buffer.clone();
        let opts = LoggingSubscriberOptions {
            format: LogFormat::Json,
            with_ansi: Some(true),
            ..LoggingSubscriberOptions::default()
        };
        // 与 fmt_layer 相同的 JSON 设置，只把输出换成内存缓冲以便断言。
        let layer = fmt::layer()
            .with_target(opts.with_target)
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_ansi(false)
            .with_writer(move || writer.clone());
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("send", tenant_id = "t1");
            let _entered = span.enter();
            tracing::info!(message_id = "m1", "persisted");
        });

        let output =
            String::from_utf8(buffer.0.lock().expect("buffer lock").clone()).expect("utf8 output");
        let line = output.lines().next().expect("one event line");
        assert!(
            !line.contains('\u{1b}'),
            "JSON logs must not carry ANSI escapes"
        );
        let value: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
        assert_eq!(value["message"], "persisted");
        assert_eq!(value["message_id"], "m1");
        assert_eq!(value["span"]["tenant_id"], "t1");
        assert_eq!(value["level"], "INFO");
    }

    #[test]
    fn fmt_layer_builds_for_both_formats() {
        for format in [LogFormat::Text, LogFormat::Json] {
            let opts = LoggingSubscriberOptions {
                format,
                ..LoggingSubscriberOptions::default()
            };
            let subscriber = tracing_subscriber::registry().with(fmt_layer(&opts, false));
            tracing::subscriber::with_default(subscriber, || tracing::debug!("smoke"));
        }
    }
}
