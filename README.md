# Flare Server Core

[![Crates.io](https://img.shields.io/crates/v/flare-server-core.svg)](https://crates.io/crates/flare-server-core)
[![Documentation](https://docs.rs/flare-server-core/badge.svg)](https://docs.rs/flare-server-core)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org/)

`flare-server-core` is the server-side infrastructure toolkit used by Flare IM
services. It packages reusable runtime, transport, messaging, authentication,
context propagation, service discovery, telemetry, and error-handling building
blocks into a small set of composable crates.

The package is business-neutral: it does not own IM product rules, inbox sync
policy, moderation, or tenant-specific workflows. Those belong in application
services and higher-level domain crates.

API documentation: [docs.rs/flare-server-core](https://docs.rs/flare-server-core)

## Installation

```toml
[dependencies]
flare-server-core = "1.0.1"
```

Feature-focused examples:

```toml
# HTTP + auth + telemetry
flare-server-core = { version = "1.0.1", features = ["http", "auth", "telemetry"] }

# gRPC service with discovery
flare-server-core = { version = "1.0.1", features = ["grpc", "discovery"] }

# Eventing and MQ integrations
flare-server-core = { version = "1.0.1", features = ["nats", "kafka"] }

# Everything
flare-server-core = { version = "1.0.1", features = ["full"] }
```

The workspace also publishes the lower-level crates with the same version:

| Crate | Purpose |
|-------|---------|
| `flare-core-base` | Context, errors, configuration, IDs, and shared types. |
| `flare-core-runtime` | Service lifecycle, task orchestration, health, shutdown, and state tracking. |
| `flare-core-infra` | KV, token validation, auth helpers, and telemetry setup. |
| `flare-core-transport` | HTTP, gRPC, service discovery, and transport middleware. |
| `flare-core-messaging` | Event bus, topic bus, NATS, Kafka, producers, consumers, and retry helpers. |
| `flare-server-core` | Aggregated re-export crate for server applications. |

All of them use version `1.0.1` so application teams can keep dependency
versions aligned.

## Feature Flags

| Feature | Description |
|---------|-------------|
| `http` | Axum HTTP helpers, response models, and middleware. |
| `grpc` | Tonic gRPC client/server context utilities and middleware. |
| `discovery` | Service discovery and client-side service selection. |
| `nats` | NATS JetStream producer/consumer support. |
| `kafka` | Kafka producer/consumer support. |
| `kv` | Infrastructure KV abstractions. |
| `auth` | Token validation, principal model, and composite validators. |
| `telemetry` | Tracing subscriber and OpenTelemetry helpers. |
| `proto` | Optional bridge to `flare-proto` structured payloads. |
| `full` | Enables all public server-core capabilities. |

## Architecture

```text
Application service
      |
flare-server-core        re-export layer for service code
      |
+-------------------+---------------------+--------------------+
| flare-core-base   | flare-core-runtime  | flare-core-infra   |
| context/errors/id | lifecycle/tasks     | auth/kv/telemetry  |
+-------------------+---------------------+--------------------+
      |
+------------------------+------------------------+
| flare-core-transport   | flare-core-messaging   |
| HTTP/gRPC/discovery    | eventbus/MQ/NATS/Kafka |
+------------------------+------------------------+
```

## Runtime Example

```rust,no_run
use flare_server_core::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::new("gateway")
        .with_address("0.0.0.0:8080".parse()?)
        .add_spawn("worker", async { Ok(()) })
        .run()
        .await?;

    Ok(())
}
```

## Context And Errors

```rust
use flare_server_core::{Context, ErrorBuilder, ErrorCode, Result};

fn require_tenant(ctx: &Context) -> Result<&str> {
    ctx.tenant_id().ok_or_else(|| {
        ErrorBuilder::new(ErrorCode::ConfigurationError, "tenant is required").build_error()
    })
}
```

## Release Verification

For package-level checks:

```bash
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo package -p flare-server-core
```

When publishing the full workspace, publish dependency crates first:

1. `flare-core-base`
2. `flare-core-runtime`
3. `flare-core-infra`
4. `flare-core-transport`
5. `flare-core-messaging`
6. `flare-server-core`

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
