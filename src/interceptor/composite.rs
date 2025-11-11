use std::sync::Arc;

use super::{AuthInterceptor, LoggingInterceptor, TracingInterceptor};
use crate::auth::TokenService;
use tonic::{Request, Status};

/// 组合拦截器
pub struct CompositeInterceptor {
    auth: Option<AuthInterceptor>,
    tracing: Option<TracingInterceptor>,
    logging: Option<LoggingInterceptor>,
}

impl CompositeInterceptor {
    pub fn new() -> Self {
        Self {
            auth: None,
            tracing: None,
            logging: None,
        }
    }

    pub fn with_auth(mut self, token_service: Arc<TokenService>) -> Self {
        self.auth = Some(AuthInterceptor::new(token_service));
        self
    }

    pub fn with_tracing(mut self) -> Self {
        self.tracing = Some(TracingInterceptor::new());
        self
    }

    pub fn with_logging(mut self) -> Self {
        self.logging = Some(LoggingInterceptor::new());
        self
    }

    pub fn intercept<T>(&self, mut req: Request<T>) -> Result<Request<T>, Status> {
        // 按顺序应用拦截器
        if let Some(ref auth) = self.auth {
            req = auth.intercept(req)?;
        }

        if let Some(ref tracing) = self.tracing {
            req = tracing.intercept(req)?;
        }

        if let Some(ref logging) = self.logging {
            req = logging.intercept(req)?;
        }

        Ok(req)
    }
}

impl Default for CompositeInterceptor {
    fn default() -> Self {
        Self::new()
    }
}
