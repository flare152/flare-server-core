# Flare Core Runtime - Unified Runtime Framework

English · [中文](README.zh-CN.md)

[![Rust](https://img.shields.io/badge/rust-1.94.0%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A **powerful, stable, and general-purpose** Rust runtime framework for high-performance IM servers supporting 1 billion+ online users.

## 🎯 Core Features

### Unified Service Startup
- ✅ **HTTP services**: axum, volo-http, actix-web
- ✅ **gRPC services**: tonic, volo
- ✅ **MQ consumers**: Kafka, RocketMQ
- ✅ **Custom tasks**: any async task
- ✅ **Scheduled tasks**: Cron expression scheduling

### Graceful Shutdown
- ✅ **Multiple signal sources**: Ctrl+C, SIGTERM, SIGINT, custom channels
- ✅ **Dependency-ordered shutdown**: shut down in reverse dependency order
- ✅ **Timeout-based forced termination**: configurable timeout duration

### Service Orchestration
- ✅ **Task dependency management**: topological sorting, circular dependency detection
- ✅ **Health checks**: periodic checks, failure thresholds
- ✅ **Service registration/deregistration**: Consul, Etcd, Nacos

### State Monitoring
- ✅ **Task state tracking**: real-time state, event notifications
- ✅ **Metrics exposure**: Prometheus format
- ✅ **Log tracing**: structured logging

### Extensibility
- ✅ **Pluggable architecture**: lifecycle hooks
- ✅ **Middleware chain**: insert logic before and after task execution
- ✅ **Custom adapters**: support for unsupported frameworks

### Simplified Mode
- ✅ **MQ consumer runner**: no service name or address required
- ✅ **Custom task runner**: concise task management
- ✅ **Flexible choice**: full mode vs. simplified mode

## 📦 Architecture Design

### Separation of Responsibilities

`flare-core-runtime` **provides only the specification** (trait definitions, configuration, core abstractions); concrete implementations are provided by other crates:

- `flare-core-transport` - HTTP/gRPC adapter implementations
- `flare-core-messaging` - MQ consumer adapter implementations

### Core Abstractions

| Trait | Description |
|-------|-------------|
| `Task` | Task abstraction; all tasks must implement it |
| `ShutdownSignal` | Shutdown signal abstraction |
| `ServiceRegistry` | Service registration abstraction |
| `HealthCheck` | Health check abstraction |
| `Plugin` | Plugin abstraction |
| `Middleware` | Middleware abstraction |
| `MetricsCollector` | Metrics collection abstraction |

## 🚀 Quick Start

### Add Dependency

```toml
[dependencies]
flare-core-runtime = { version = "0.2", path = "path/to/flare-core-runtime" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

### Basic Example

```rust
use flare_core_runtime::ServiceRuntime;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create the runtime
    let runtime = ServiceRuntime::new("my-service")
        .with_address("0.0.0.0:8080".parse().unwrap())
        .add_spawn("grpc-server", async {
            // Start the gRPC service
            Ok(())
        })
        .add_spawn("kafka-consumer", async {
            // Start the Kafka consumer
            Ok(())
        });

    // Run (wait for Ctrl+C)
    runtime.run().await?;

    Ok(())
}
```

### Simplified Mode Examples

#### Run MQ Consumers Only

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::mq_consumer()
        .add_spawn("kafka-consumer", async {
            // Consume Kafka messages
            Ok(())
        })
        .add_spawn("nats-consumer", async {
            // Consume NATS messages
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

#### Run Custom Tasks Only

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::tasks()
        .add_spawn("data-sync", async {
            // Data synchronization task
            Ok(())
        })
        .add_spawn("cache-refresh", async {
            // Cache refresh task
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

For more simplified-mode usage, see [SIMPLE_MODE.md](SIMPLE_MODE.md).


### With Service Registration

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = ServiceRuntime::new("my-service")
        .with_address("0.0.0.0:8080".parse().unwrap())
        .add_spawn("grpc", async { Ok(()) });

    // With service registration
    runtime.run_with_registration(|addr| {
        Box::pin(async move {
            // Register to Consul/Etcd/Nacos
            println!("Registering service at {}", addr);
            Ok(None) // Return the registrar (optional)
        })
    }).await?;

    Ok(())
}
```

### Task Dependencies

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = ServiceRuntime::new("my-service")
        // Start the database first
        .add_spawn("database", async {
            println!("Database started");
            Ok(())
        })
        // Then start the cache (depends on the database)
        .add_spawn_with_deps("cache", async {
            println!("Cache started");
            Ok(())
        }, vec!["database".to_string()])
        // Finally start gRPC (depends on the cache)
        .add_spawn_with_deps("grpc", async {
            println!("gRPC started");
            Ok(())
        }, vec!["cache".to_string()]);

    runtime.run().await?;
    Ok(())
}
```

### Custom Task

```rust
use flare_core_runtime::task::{Task, TaskResult};
use std::pin::Pin;
use std::future::Future;

struct MyCustomTask {
    name: String,
}

impl Task for MyCustomTask {
    fn name(&self) -> &str {
        &self.name
    }

    fn run(
        self: Box<Self>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
        Box::pin(async move {
            // Task logic
            tokio::select! {
                _ = async {
                    // Main logic
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                } => {}
                _ = shutdown_rx => {
                    println!("Shutdown signal received");
                }
            }
            Ok(())
        })
    }
}
```

## 📊 State Monitoring

### Subscribe to State Events

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = ServiceRuntime::new("my-service")
        .add_spawn("task-1", async { Ok(()) });

    // Get the state tracker
    let tracker = runtime.state_tracker();

    // Subscribe to state events
    let mut rx = tracker.subscribe();

    tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            println!(
                "Task {} state changed: {:?} -> {:?}",
                event.task_name, event.old_state, event.new_state
            );
        }
    });

    runtime.run().await?;
    Ok(())
}
```

## 🔧 Configuration

```rust
use flare_core_runtime::{ServiceRuntime, RuntimeConfig};
use std::time::Duration;

let config = RuntimeConfig::new()
    .with_shutdown_timeout(Duration::from_secs(10))
    .with_task_startup(
        RuntimeConfig::new().task_startup.clone()
            .with_concurrency(8)
            .with_ready_check_timeout(Duration::from_secs(60))
    );

let runtime = ServiceRuntime::new("my-service")
    .with_config(config);
```

## 🎨 Technical Highlights

1. **Rust 2024 native async fn in traits** - no async-trait macro used
2. **Zero-cost abstractions** - all extension points defined via traits
3. **Thread-safe** - uses `Arc<RwLock>` and `broadcast` channels
4. **Event-driven** - state changes automatically emit events
5. **Standardized error handling** - all errors defined with `thiserror`
6. **Builder pattern** - all configurations provide Builder methods
7. **Complete documentation** - all types have doc comments and examples
8. **Test coverage** - all core components have unit tests

## 📚 API Documentation

Run `cargo doc --open` to view the complete API documentation.

## 🤝 Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

Thanks to the following projects for the inspiration:
- [Tokio](https://tokio.rs/) - async runtime
- [Tonic](https://github.com/hyperium/tonic) - gRPC framework
- [Axum](https://docs.rs/axum/) - Web framework
