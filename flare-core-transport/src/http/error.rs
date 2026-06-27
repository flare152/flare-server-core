use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use flare_core_base::error::{ErrorBuilder, ErrorCode, FlareError};
use thiserror::Error;

use super::response::ApiResponse;

#[derive(Debug, Error, Clone)]
#[error("{0}")]
pub struct HttpApiError(pub FlareError);

pub type Result<T> = std::result::Result<T, HttpApiError>;

impl IntoResponse for HttpApiError {
    fn into_response(self) -> Response {
        let status = status_from_flare_error(&self.0);
        let body: ApiResponse<()> = ApiResponse::from(self.0);
        (status, Json(body)).into_response()
    }
}

impl HttpApiError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self(
            ErrorBuilder::new(ErrorCode::HttpNotFound, "NOT_FOUND")
                .details(message)
                .build_error(),
        )
    }

    pub fn bad_request(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self(
            ErrorBuilder::new(ErrorCode::HttpBadRequest, reason)
                .details(message)
                .build_error(),
        )
    }

    pub fn internal(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self(
            ErrorBuilder::new(ErrorCode::HttpInternalServerError, reason)
                .details(message)
                .build_error(),
        )
    }
}

fn status_from_flare_error(error: &FlareError) -> StatusCode {
    match error.code() {
        Some(ErrorCode::HttpBadRequest) => StatusCode::BAD_REQUEST,
        Some(ErrorCode::HttpUnauthorized) => StatusCode::UNAUTHORIZED,
        Some(ErrorCode::HttpForbidden) => StatusCode::FORBIDDEN,
        Some(ErrorCode::HttpNotFound) => StatusCode::NOT_FOUND,
        Some(ErrorCode::HttpMethodNotAllowed) => StatusCode::METHOD_NOT_ALLOWED,
        Some(ErrorCode::HttpRequestTimeout) => StatusCode::REQUEST_TIMEOUT,
        Some(ErrorCode::HttpConflict) => StatusCode::CONFLICT,
        Some(ErrorCode::HttpUnprocessableEntity) => StatusCode::UNPROCESSABLE_ENTITY,
        Some(ErrorCode::HttpTooManyRequests) => StatusCode::TOO_MANY_REQUESTS,
        Some(ErrorCode::HttpBadGateway) => StatusCode::BAD_GATEWAY,
        Some(ErrorCode::HttpServiceUnavailable) => StatusCode::SERVICE_UNAVAILABLE,
        Some(ErrorCode::HttpGatewayTimeout) => StatusCode::GATEWAY_TIMEOUT,
        Some(ErrorCode::HttpInternalServerError) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl From<serde_json::Error> for HttpApiError {
    fn from(err: serde_json::Error) -> Self {
        HttpApiError::bad_request("SERIALIZATION_ERROR", err.to_string())
    }
}

#[cfg(feature = "grpc")]
impl From<tonic::Status> for HttpApiError {
    fn from(status: tonic::Status) -> Self {
        HttpApiError(
            ErrorBuilder::new(ErrorCode::HttpBadGateway, "GRPC_ERROR")
                .details(status.message())
                .build_error(),
        )
    }
}

impl From<anyhow::Error> for HttpApiError {
    fn from(err: anyhow::Error) -> Self {
        HttpApiError::internal("INTERNAL_ERROR", err.to_string())
    }
}

impl From<FlareError> for HttpApiError {
    fn from(err: FlareError) -> Self {
        HttpApiError(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_keeps_flare_error_message() {
        let err = HttpApiError::bad_request("VALIDATION_ERROR", "invalid parameter");
        assert!(err.to_string().contains("错误 [BAD_REQUEST]"));
    }
}
