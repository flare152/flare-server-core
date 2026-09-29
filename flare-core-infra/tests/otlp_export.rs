//! OTLP trace 导出端到端：本地 gRPC 收集端 + 真实全局 subscriber。
//!
//! 独立测试二进制，全局 subscriber 只在本进程安装一次。证明 JSON 日志与 OTLP 同时开启时，
//! span 确实送达收集端、resource 带配置的服务名，且 `shutdown_tracing` 不等批处理周期即完成导出。
#![cfg(feature = "otel")]

use std::time::Duration;

use flare_core_infra::telemetry::{
    LogFormat, LoggingSubscriberOptions, OtlpTracingOptions, init_tracing_subscriber,
    shutdown_tracing,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
    trace_service_server::{TraceService, TraceServiceServer},
};
use tokio::sync::mpsc;
use tonic::{Request, Response, Status};

struct Collector(mpsc::UnboundedSender<ExportTraceServiceRequest>);

#[tonic::async_trait]
impl TraceService for Collector {
    async fn export(
        &self,
        request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        let _ = self.0.send(request.into_inner());
        Ok(Response::new(ExportTraceServiceResponse::default()))
    }
}

fn service_name(request: &ExportTraceServiceRequest) -> Vec<String> {
    request
        .resource_spans
        .iter()
        .filter_map(|spans| spans.resource.as_ref())
        .flat_map(|resource| resource.attributes.iter())
        .filter(|attribute| attribute.key == "service.name")
        .filter_map(|attribute| attribute.value.as_ref())
        .filter_map(|value| match &value.value {
            Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(name)) => {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

fn span_names(request: &ExportTraceServiceRequest) -> Vec<String> {
    request
        .resource_spans
        .iter()
        .flat_map(|resource| resource.scope_spans.iter())
        .flat_map(|scope| scope.spans.iter())
        .map(|span| span.name.clone())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spans_reach_otlp_collector_with_json_logs_enabled() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind collector");
    let address = listener.local_addr().expect("collector address");
    let (sender, mut received) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(TraceServiceServer::new(Collector(sender)))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener)),
    );

    let logging = LoggingSubscriberOptions {
        level: "info".to_string(),
        format: LogFormat::Json,
        ..LoggingSubscriberOptions::default()
    };
    let otlp = OtlpTracingOptions::new("otlp-probe", format!("http://{address}"));
    init_tracing_subscriber(Some(&logging), Some(&otlp)).expect("install subscriber");

    {
        let span = tracing::info_span!("otlp_probe_span", tenant_id = "t1");
        let _entered = span.enter();
        tracing::info!("inside probe span");
    }

    // 不等 5 秒的批处理周期：shutdown_tracing 必须立即把队列里的 span 导出。
    // 批处理线程通过 tokio 通道发 gRPC，放到阻塞线程里调用以免占住运行时。
    let flushed = tokio::task::spawn_blocking(|| shutdown_tracing(Duration::from_secs(5)));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut names = Vec::new();
    let mut services = Vec::new();
    while !names.iter().any(|name| name == "otlp_probe_span") {
        let request = tokio::time::timeout_at(deadline, received.recv())
            .await
            .expect("collector received spans before deadline")
            .expect("collector channel open");
        names.extend(span_names(&request));
        services.extend(service_name(&request));
    }
    assert!(
        services.iter().any(|name| name == "otlp-probe"),
        "{services:?}"
    );
    flushed
        .await
        .expect("shutdown task joins")
        .expect("exporter shut down cleanly");
    server.abort();
}
