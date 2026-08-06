# flare-core-base

English · [中文](README.zh-CN.md)

`flare-core-base` is the foundational capability library of `flare-server-core`, providing common types and infrastructure capabilities reusable across services.

## Main Responsibilities

- **Context**: Request context and trace propagation (`trace_id` / `request_id` / `tenant_id` / `user_id`, etc.).
- **Error**: Unified error model, error codes, builders, and gRPC-friendly conversions.
- **Config**: Base configuration structures and layered configuration reading.
- **I18n**: Default Chinese/English messages and internationalization support.
- **Types/Utils**: Common types and general-purpose utility functions.

## Module Structure

- `src/context`: Context object, type-safe extension fields, and helper utilities.
- `src/error`: `FlareError`, `ErrorCode`, error builders, and conversions.
- `src/config`: Configuration structs and layered readers.
- `src/i18n`: Default translations and language resource interfaces.
- `src/types`: Base type definitions for services.
- `src/utils`: Cross-module utility functions.

## Configuration Scheme (Env First, TOML as Fallback)

Currently provides `config::LayeredConfig`:

- Reads environment variables first;
- Falls back to TOML when environment variables are absent;
- Suitable as a unified cross-service configuration injection mechanism (not bound to any specific business configuration struct).

Example:

```rust
use std::path::Path;
use flare_core_base::config::LayeredConfig;

let layered = LayeredConfig::from_optional_toml(Some(Path::new("config/services/capability.toml")));

let timeout_ms = layered
    .resolve_u64("FLARE_CAPABILITY_PLUGIN_CALL_TIMEOUT_MS", "capability_runtime.plugin_call_timeout_ms")
    .unwrap_or(5000);
```

## Design Principles

- **Sink mechanisms, float business logic**: Extract only general-purpose mechanisms, do not carry business models.
- **Explicit priority**: Environment variables override TOML, making behavior predictable.
- **Zero-intrusion reuse**: Provided to each service on demand through generics/base APIs.
- **Stable interfaces**: Prioritize backward stability across crates.

## Applicable Scenarios

- Multiple services sharing a single set of configuration loading rules;
- Unified error and context models;
- Reducing the cost of each service reimplementing base logic.
