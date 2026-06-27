//! HTTP 模块

pub mod adapter;
pub mod context;
pub mod error;
pub mod middleware;
pub mod response;

pub use adapter::{HttpAdapter, HttpAdapterBuilder};
pub use context::ContextFromHeaders;
pub use error::{HttpApiError, Result};
pub use middleware::*;
pub use response::*;
