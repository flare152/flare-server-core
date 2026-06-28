//! Transport infrastructure for Flare server applications.
//!
//! `flare-core-transport` contains optional HTTP, gRPC, and service-discovery
//! integrations. It is intentionally feature-gated so services can depend only
//! on the transport surfaces they actually use.
//!
//! # Features
//!
//! - `grpc`: tonic gRPC context, client, and middleware helpers.
//! - `http`: Axum response and middleware helpers.
//! - `discovery`: service discovery backends and service clients.
//!
//! # Example
//!
//! ```toml
//! [dependencies]
//! flare-core-transport = { version = "1.0.1", features = ["grpc", "http"] }
//! ```

// gRPC integration.
#[cfg(feature = "grpc")]
pub mod grpc;

// HTTP integration.
#[cfg(feature = "http")]
pub mod http;

// Service discovery integration.
#[cfg(feature = "discovery")]
pub mod discovery;

#[cfg(feature = "discovery")]
pub use discovery::{
    DiscoveryBackend, DiscoveryConfig, DiscoveryFactory, ServiceDiscover, ServiceInstance,
};
