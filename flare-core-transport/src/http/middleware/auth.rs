//! HTTP 认证中间件

use axum::{
    Json,
    extract::{Extension, Request},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::http::response::ApiResponse;
use flare_core_base::context::keys;
use flare_core_base::error::ErrorCode;
use flare_core_infra::auth::TokenService;

/// 认证中间件
///
/// 使用 TokenService 验证 JWT Token 并将用户信息注入到请求 Header 中
///
/// # Example
///
/// ```rust
/// use axum::middleware;
/// use flare_server_core::http::middleware::auth_middleware;
/// use flare_server_core::auth::TokenService;
///
/// let token_service = Arc::new(TokenService::new("secret", "issuer", 3600));
///
/// let app = Router::new()
///     .route("/api/protected", post(handler))
///     .layer(Extension(token_service))
///     .route_layer(middleware::from_fn(auth_middleware));
/// ```
pub async fn auth_middleware(
    Extension(token_service): Extension<Arc<TokenService>>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();

    // 1. 提取 Authorization Header
    let auth_header = headers.get("Authorization").and_then(|h| h.to_str().ok());

    if auth_header.is_none() {
        warn!("Missing Authorization header");
        let response: ApiResponse<()> = ApiResponse::from_code(ErrorCode::HttpUnauthorized);
        return (StatusCode::UNAUTHORIZED, Json(response)).into_response();
    }

    // 2. 验证 Bearer Token 格式
    let auth_header = auth_header.unwrap();
    if !auth_header.starts_with("Bearer ") {
        warn!("Invalid Authorization header format");
        let response: ApiResponse<()> = ApiResponse::from_code(ErrorCode::HttpUnauthorized);
        return (StatusCode::UNAUTHORIZED, Json(response)).into_response();
    }

    let token = &auth_header[7..]; // Skip "Bearer "

    // 3. 验证 Token
    if token.is_empty() {
        warn!("Empty token");
        let response: ApiResponse<()> = ApiResponse::from_code(ErrorCode::HttpUnauthorized);
        return (StatusCode::UNAUTHORIZED, Json(response)).into_response();
    }

    // 4. 使用 TokenService 验证 JWT
    match token_service.validate_token(token) {
        Ok(claims) => {
            debug!(
                user_id = %claims.sub,
                device_id = ?claims.device_id,
                tenant_id = ?claims.tenant_id,
                "Token validated successfully"
            );

            // 5. 将用户信息注入到请求 Header 中
            let mut request = request;
            let headers = request.headers_mut();

            // 注入用户 ID
            if let Ok(user_id) = HeaderValue::from_str(&claims.sub) {
                headers.insert(keys::USER_ID, user_id);
            }

            // 注入设备 ID
            if let Some(device_id) = &claims.device_id {
                if let Ok(device_id_value) = HeaderValue::from_str(device_id) {
                    headers.insert(keys::DEVICE_ID, device_id_value);
                }
            }

            // 注入租户 ID
            if let Some(tenant_id) = &claims.tenant_id {
                if let Ok(tenant_id_value) = HeaderValue::from_str(tenant_id) {
                    headers.insert(keys::TENANT_ID, tenant_id_value);
                }
            }

            next.run(request).await
        }
        Err(e) => {
            warn!(error = %e, "Token validation failed");
            let response: ApiResponse<()> = ApiResponse::from_code(ErrorCode::HttpUnauthorized);
            (StatusCode::UNAUTHORIZED, Json(response)).into_response()
        }
    }
}

/// 可选认证中间件
///
/// 与 auth_middleware 类似,但允许无 Token 的请求通过
/// 如果提供了 Token 则验证,否则继续执行
pub async fn optional_auth_middleware(
    Extension(token_service): Extension<Arc<TokenService>>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();

    // 1. 提取 Authorization Header
    let auth_header = headers.get("Authorization").and_then(|h| h.to_str().ok());

    // 如果没有 Authorization header,直接通过
    let Some(auth_header) = auth_header else {
        return next.run(request).await;
    };

    // 2. 解析 Bearer Token
    let token = if auth_header.starts_with("Bearer ") {
        &auth_header[7..]
    } else {
        // Token 格式错误,但这是可选认证,所以继续执行
        return next.run(request).await;
    };

    // 3. 使用 TokenService 验证 JWT
    match token_service.validate_token(token) {
        Ok(claims) => {
            debug!(
                user_id = %claims.sub,
                device_id = ?claims.device_id,
                tenant_id = ?claims.tenant_id,
                "Token validated successfully"
            );

            // 4. 将用户信息注入到请求 Header 中
            let mut request = request;
            let headers = request.headers_mut();

            if let Ok(user_id) = HeaderValue::from_str(&claims.sub) {
                headers.insert(keys::USER_ID, user_id);
            }

            if let Some(device_id) = &claims.device_id {
                if let Ok(device_id_value) = HeaderValue::from_str(device_id) {
                    headers.insert(keys::DEVICE_ID, device_id_value);
                }
            }

            if let Some(tenant_id) = &claims.tenant_id {
                if let Ok(tenant_id_value) = HeaderValue::from_str(tenant_id) {
                    headers.insert(keys::TENANT_ID, tenant_id_value);
                }
            }

            next.run(request).await
        }
        Err(e) => {
            // Token 验证失败,但这是可选认证,记录警告后继续执行
            warn!(error = %e, "Token validation failed in optional auth, continuing");
            next.run(request).await
        }
    }
}
