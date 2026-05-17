//! HTTP 追踪中间件

use axum::{extract::Request, middleware::Next, response::Response};
use tracing::debug_span;

/// 追踪中间件
pub async fn tracing_middleware(request: Request, next: Next) -> Response {
    let span = debug_span!(
        "http_request",
        method = %request.method(),
        uri = %request.uri(),
    );

    let _enter = span.enter();
    next.run(request).await
}
