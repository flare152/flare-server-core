//! Service runtime and lifecycle orchestration for Flare services.
//!
//! `flare-core-runtime` manages service tasks, dependency ordering, health
//! checks, shutdown signals, plugin hooks, metrics collection, middleware, and
//! state tracking. It is transport-neutral and is used by HTTP, gRPC, MQ, and
//! custom workers.
//!
//! # 核心特性
//!
//! - Unified service startup for HTTP, gRPC, MQ consumers, and custom tasks.
//! - Graceful shutdown with multiple signal sources and dependency ordering.
//! - Service orchestration with task dependencies, health checks, and state
//!   tracking.
//! - Extensible plugin hooks, middleware chains, and custom adapters.
//!
//! # Architecture
//!
//! `flare-core-runtime` provides transport-neutral traits, configuration, and
//! orchestration primitives. Concrete adapters live in sibling crates:
//!
//! - `flare-core-transport` implements HTTP and gRPC adapters.
//! - `flare-core-messaging` implements MQ consumer adapters.
//!
//! # Example
//!
//! ```rust,no_run
//! use flare_core_runtime::{ServiceRuntime, Task, TaskResult};
//! use std::pin::Pin;
//! use std::future::Future;
//!
//! // Define a custom task.
//! struct MyTask {
//!     name: String,
//! }
//!
//! impl Task for MyTask {
//!     fn name(&self) -> &str {
//!         &self.name
//!     }
//!
//!     fn run(
//!         self: Box<Self>,
//!         shutdown_rx: tokio::sync::oneshot::Receiver<()>,
//!     ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
//!         Box::pin(async move {
//!             // Task logic.
//!             Ok(())
//!         })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create the runtime.
//!     let runtime = ServiceRuntime::new("my-service")
//!         .add_task(Box::new(MyTask { name: "task-1".to_string() }));
//!
//!     // Run until shutdown.
//!     runtime.run().await?;
//!     Ok(())
//! }
//! ```

// Module declarations.
pub mod config;
pub mod error;
pub mod health;
pub mod metrics;
pub mod middleware;
pub mod plugin;
pub mod registry;
pub mod runtime;
pub mod signal;
pub mod state;
pub mod task;
pub mod utils;

// Re-exports
pub use config::RuntimeConfig;
pub use error::{
    HealthError, MetricsError, MiddlewareError, PluginError, RegistryError, RuntimeError,
};
pub use health::{HealthCheck, HealthCheckResult, HealthChecker};
pub use metrics::MetricsCollector;
pub use middleware::{Middleware, MiddlewareChain};
pub use plugin::{Plugin, PluginContext, PluginManager};
pub use registry::{ServiceInfo, ServiceRegistry};
pub use runtime::{HealthFailureAction, ServiceRuntime};
pub use signal::{
    ChannelSignal, CompositeSignal, CtrlCSignal, ShutdownSignal, UnixSignal, UnixSignalKind,
};
pub use state::{RuntimeEvent, StateEvent, StateTracker, TaskStateInfo};
pub use task::{SpawnTask, Task, TaskManager, TaskResult, TaskState};
pub use utils::topological_sort;
