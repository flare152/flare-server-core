use axum::http::HeaderMap;
use flare_core_base::context::{Context, Ctx, keys};
use std::sync::Arc;
use uuid::Uuid;

/// 从 HTTP headers 构建统一 Context。
pub trait ContextFromHeaders {
    fn from_headers(headers: &HeaderMap) -> Self;
    fn require_user_id(&self) -> Result<&str, &'static str>;
}

impl ContextFromHeaders for Ctx {
    fn from_headers(headers: &HeaderMap) -> Self {
        let request_id =
            header_value(headers, keys::REQUEST_ID).unwrap_or_else(|| Uuid::new_v4().to_string());
        let trace_id =
            header_value(headers, keys::TRACE_ID).unwrap_or_else(|| Uuid::new_v4().to_string());

        let mut ctx = Context::with_request_id(request_id).with_trace_id(trace_id);

        if let Some(user_id) = header_value(headers, keys::USER_ID) {
            ctx = ctx.with_user_id(user_id);
        }
        if let Some(tenant_id) = header_value(headers, keys::TENANT_ID) {
            ctx = ctx.with_tenant_id(tenant_id);
        }
        if let Some(device_id) = header_value(headers, keys::DEVICE_ID) {
            ctx = ctx.with_device_id(device_id);
        }
        if let Some(platform) = header_value(headers, keys::PLATFORM)
            .or_else(|| header_value(headers, "x-device-platform"))
        {
            ctx = ctx.with_platform(platform);
        }
        if let Some(session_id) = header_value(headers, keys::SESSION_ID) {
            ctx = ctx.with_session_id(session_id);
        }

        Arc::new(ctx)
    }

    fn require_user_id(&self) -> Result<&str, &'static str> {
        self.user_id()
            .filter(|value| !value.is_empty())
            .ok_or("User ID is required but not present in context")
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_from_headers_keeps_identity_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert(keys::TRACE_ID, "trace-789".parse().unwrap());
        headers.insert(keys::REQUEST_ID, "request-789".parse().unwrap());
        headers.insert(keys::USER_ID, "user-012".parse().unwrap());

        let ctx = Ctx::from_headers(&headers);

        assert_eq!(ctx.trace_id(), "trace-789");
        assert_eq!(ctx.request_id(), "request-789");
        assert_eq!(ctx.user_id(), Some("user-012"));
    }
}
