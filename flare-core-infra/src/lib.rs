//! Infrastructure adapters for Flare server applications.
//!
//! `flare-core-infra` collects reusable infrastructure helpers that are shared
//! across services: token validation, authenticated principals, KV storage
//! traits, metrics, and tracing subscriber setup.

pub mod auth;
pub mod kv;
pub mod metrics;
pub mod telemetry;

// KV re-exports.
pub use kv::{KvBackend, KvEntry, KvError, KvStore};

// Auth re-exports.
pub use auth::{
    AuthError, AuthenticatedPrincipal, CompositeTokenValidator, TokenClaims, TokenService,
    TokenValidationRequest, TokenValidator, TrustedIssuer,
};

// Telemetry re-exports.
pub use telemetry::{LoggingSubscriberOptions, init_fmt_subscriber};
