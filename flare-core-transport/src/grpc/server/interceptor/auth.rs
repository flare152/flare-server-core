use std::sync::Arc;

use crate::auth::TokenService;
use tonic::{Request, Status};
use tracing::{error, info};

/// 认证拦截器
pub struct AuthInterceptor {
    token_service: Arc<TokenService>,
}

impl AuthInterceptor {
    pub fn new(token_service: Arc<TokenService>) -> Self {
        Self { token_service }
    }

    pub fn intercept<T>(&self, req: Request<T>) -> Result<Request<T>, Status> {
        let metadata = req.metadata();

        // 提取 token
        let token = metadata
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).to_string())
            .ok_or_else(|| Status::unauthenticated("Missing authorization token"))?;

        // 验证 token
        match self.token_service.validate_token(&token) {
            Ok(claims) => {
                info!(subject = %claims.sub, "Request authenticated");
                Ok(req)
            }
            Err(err) => {
                error!(?err, "Invalid token");
                Err(Status::unauthenticated("Invalid token"))
            }
        }
    }
}
