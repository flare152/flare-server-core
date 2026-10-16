//! Narrow internal route; Edge public servers MUST deny /internal/**.
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use flare_core_infra::ops::{Refusal, RuntimeProbe};
use std::sync::Arc;

#[derive(Clone)]
struct ProbeState {
    probe: Arc<RuntimeProbe>,
    permits: Arc<tokio::sync::Semaphore>,
}
pub fn router(probe: Option<RuntimeProbe>) -> Router {
    let Some(probe) = probe else {
        return Router::new();
    };
    Router::new()
        .route("/internal/ops/runtime", any(receipt))
        .with_state(ProbeState {
            probe: Arc::new(probe),
            permits: Arc::new(tokio::sync::Semaphore::new(2)),
        })
}
async fn receipt(State(state): State<ProbeState>, request: Request) -> Response {
    let mut response = serve(state, request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
async fn serve(state: ProbeState, request: Request) -> Response {
    let Ok(_permit) = state.permits.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let headers = request.headers();
    if request.method() != Method::GET
        || request.uri().query().is_some()
        || headers.contains_key(header::ORIGIN)
        || headers.contains_key(header::TRANSFER_ENCODING)
        || headers
            .get(header::CONTENT_LENGTH)
            .is_some_and(|v| v != "0")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let one = |key: &str| {
        if headers.get_all(key).iter().count() == 1 {
            headers.get(key).and_then(|v| v.to_str().ok())
        } else {
            None
        }
    };
    let result = state.probe.receipt(
        one("x-flare-ops-token").unwrap_or(""),
        one("x-flare-ops-nonce").unwrap_or(""),
    );
    if result.is_ok()
        && !matches!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                axum::body::to_bytes(request.into_body(), 0)
            )
            .await,
            Ok(Ok(_))
        )
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match result {
        Ok(value) => Json(value).into_response(),
        Err(Refusal::Unauthorized) => StatusCode::UNAUTHORIZED.into_response(),
        Err(Refusal::InvalidNonce) => StatusCode::BAD_REQUEST.into_response(),
        Err(Refusal::Unavailable) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use std::collections::BTreeMap;
    use tower::ServiceExt;
    fn app() -> Router {
        router(Some(
            RuntimeProbe::new("core-admin", &"a".repeat(64), || {
                Some(BTreeMap::from([(
                    "context".into(),
                    flare_core_infra::ops::LoadedKeys {
                        keys: BTreeMap::from([("gateway".into(), "b".repeat(64))]),
                        fallback: false,
                    },
                )]))
            })
            .unwrap(),
        ))
    }
    #[tokio::test]
    async fn route_uses_private_capability_and_echoes_fresh_nonce_only() {
        for (method, uri, token, extra, status) in [
            ("GET", "/internal/ops/runtime", "a".repeat(64), None, 200),
            ("GET", "/internal/ops/runtime", "wrong".into(), None, 401),
            ("POST", "/internal/ops/runtime", "a".repeat(64), None, 400),
            (
                "GET",
                "/internal/ops/runtime?x=1",
                "a".repeat(64),
                None,
                400,
            ),
            (
                "GET",
                "/internal/ops/runtime",
                "a".repeat(64),
                Some("origin"),
                400,
            ),
            (
                "GET",
                "/internal/ops/runtime",
                "a".repeat(64),
                Some("x-flare-ops-token"),
                401,
            ),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header("x-flare-ops-token", token)
                .header("x-flare-ops-nonce", "0".repeat(32))
                .body(Body::empty())
                .unwrap();
            if let Some(key) = extra {
                request
                    .headers_mut()
                    .append(key, HeaderValue::from_static("bad"));
            }
            let response = app().oneshot(request).await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = to_bytes(response.into_body(), 65536).await.unwrap();
            if status == 200 {
                let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(value["nonce"], "0".repeat(32));
                assert_eq!(value["component"], "core-admin");
                assert!(!String::from_utf8_lossy(&body).contains(&"a".repeat(64)));
            } else {
                assert!(body.is_empty());
            }
        }
        assert_eq!(
            router(None)
                .oneshot(
                    Request::builder()
                        .uri("/internal/ops/runtime")
                        .body(Body::empty())
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let request = Request::builder()
            .uri("/internal/ops/runtime")
            .header("x-flare-ops-token", "a".repeat(64))
            .header("x-flare-ops-nonce", "0".repeat(32))
            .body(Body::from("undeclared body"))
            .unwrap();
        assert_eq!(
            app().oneshot(request).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
}
