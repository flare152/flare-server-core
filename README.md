# Flare Server Core

English · [中文](README.zh-CN.md)

> ## ⚠ Clone this next to its sibling repos

This crate depends on `flare-proto` by **path**, at `../flare-proto`. Cloning this
repository on its own does not build — `cargo metadata` fails before it reaches your
code, with an error naming an internal workspace member that tells you nothing about
the real cause.

```bash
mkdir flare && cd flare
git clone https://github.com/flare-im/flare-proto.git
git clone https://github.com/flare152/flare-server-core.git
cd flare-server-core && cargo check
```


> ## ℹ 这是通信基础设施，不是开箱即用的 IM 产品
>
> 说在前面，免得你 clone 完才发现登不上去：**开源部分不含账号体系**
> （没有注册登录、好友关系、群角色/审批/禁言、朋友圈）。
>
> 但它自带完整且可插拔的鉴权契约，两条路都在开源侧：
>
> - **`CoreJwtTokenValidator`** —— 本地验 JWT。手签一个 token 就能跑起来做
>   demo / POC，**不需要任何用户体系**。
> - **`HttpHookTokenValidator`** —— 把 token POST 到你自己的接口，
>   **这是接入自有用户体系的入口**。
>
> 业务规则同理：`flare-im-core/crates/flare-im-hooks` 提供 9 个扩展点
> （PreSend / PostSend / Delivery / Recall / MessageRead / MessageReaction /
> ConversationLifecycle / ConversationMember / GetConversationParticipants）。
>
> 要上生产，你需要自行实现用户体系并按上述契约接入 —— 与 Sendbird /
> Twilio Conversations 的「自带身份」模型一致，区别是 Flare 可自托管、
> 协议与核心可审计。
>
> 边界详情见 [GOVERNANCE.md](GOVERNANCE.md)。


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

---

## 下一步

| 想做什么 | 去哪里 |
|---|---|
| **五分钟跑起来** | [QUICKSTART](https://github.com/flare-im/flare-im-core-server/blob/main/QUICKSTART.md) —— 起服务、手签 token、调通接口，**不需要自建用户体系** |
| 接入自己的用户系统 | 实现 `TokenValidator`（`CoreJwtTokenValidator` 本地验签 / `HttpHookTokenValidator` 调你的接口） |
| 加自己的业务规则 | `flare-im-hooks` 的 9 个扩展点：PreSend / PostSend / Delivery / Recall / MessageRead / MessageReaction / ConversationLifecycle / ConversationMember / GetConversationParticipants |
| 做界面 | [`@flare-im/vue-ui`](https://www.npmjs.com/package/@flare-im/vue-ui) —— 107 个组件，四端一致的契约 |
| 报安全问题 | [SECURITY.md](SECURITY.md)，**请勿开公开 issue** |

## 需要账号体系与社交能力时

开源部分是**通信基础设施**。如果你需要的是现成的账号、好友关系、群治理（角色 / 入群审批 / 禁言）、朋友圈，
这些在商业模块里 —— 自研这一层通常要数月，且都是与通信无关的重复劳动。

企业场景另有 SSO / 组织架构 / 审计导出 / 数据驻留 / SLA 支持。

咨询：`flare1522@163.com`

> 边界划分与不变承诺见 [GOVERNANCE](https://github.com/flare-im/flare-im-core-server/blob/main/GOVERNANCE.md)。
> 简言之：**已开源的不会被收回，鉴权与 hooks 契约永远开源、不会为逼迫付费而阉割。**
