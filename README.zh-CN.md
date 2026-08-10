# Flare Server Core

[English](README.md) · 中文

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

`flare-server-core` 是 Flare IM 各项服务所使用的服务端基础设施工具包。它把可复用的
运行时、传输、消息、鉴权、上下文传播、服务发现、遥测以及错误处理等构建块，打包成
一组小而可组合的 crate。

本包与业务无关：它不承载 IM 产品规则、收件箱同步策略、内容审核或租户专属的工作流。
这些应归属于应用服务和更上层的领域 crate。

API 文档：[docs.rs/flare-server-core](https://docs.rs/flare-server-core)

## 安装

```toml
[dependencies]
flare-server-core = "1.0.1"
```

按功能聚焦的示例：

```toml
# HTTP + 鉴权 + 遥测
flare-server-core = { version = "1.0.1", features = ["http", "auth", "telemetry"] }

# 带服务发现的 gRPC 服务
flare-server-core = { version = "1.0.1", features = ["grpc", "discovery"] }

# 事件与 MQ 集成
flare-server-core = { version = "1.0.1", features = ["nats", "kafka"] }

# 全部功能
flare-server-core = { version = "1.0.1", features = ["full"] }
```

该工作区还以相同版本发布了各个更底层的 crate：

| Crate | 用途 |
|-------|-------|
| `flare-core-base` | 上下文、错误、配置、ID 以及共享类型。 |
| `flare-core-runtime` | 服务生命周期、任务编排、健康检查、优雅关闭以及状态追踪。 |
| `flare-core-infra` | KV、token 校验、鉴权辅助工具以及遥测配置。 |
| `flare-core-transport` | HTTP、gRPC、服务发现以及传输中间件。 |
| `flare-core-messaging` | 事件总线、主题总线、NATS、Kafka、生产者、消费者以及重试辅助工具。 |
| `flare-server-core` | 面向服务端应用的聚合再导出 crate。 |

它们全部使用版本 `1.0.1`，以便应用团队保持依赖版本对齐。

## 功能开关（Feature Flags）

| Feature | 说明 |
|---------|-------|
| `http` | Axum HTTP 辅助工具、响应模型以及中间件。 |
| `grpc` | Tonic gRPC 客户端/服务端上下文工具以及中间件。 |
| `discovery` | 服务发现以及客户端侧的服务选择。 |
| `nats` | NATS JetStream 生产者/消费者支持。 |
| `kafka` | Kafka 生产者/消费者支持。 |
| `kv` | 基础设施 KV 抽象。 |
| `auth` | Token 校验、principal 模型以及组合校验器。 |
| `telemetry` | Tracing subscriber 以及 OpenTelemetry 辅助工具。 |
| `proto` | 到 `flare-proto` 结构化载荷的可选桥接。 |
| `full` | 启用所有公开的 server-core 能力。 |

## 架构

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

## 运行时示例

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

## 上下文与错误

```rust
use flare_server_core::{Context, ErrorBuilder, ErrorCode, Result};

fn require_tenant(ctx: &Context) -> Result<&str> {
    ctx.tenant_id().ok_or_else(|| {
        ErrorBuilder::new(ErrorCode::ConfigurationError, "tenant is required").build_error()
    })
}
```

## 发布校验

包级别的检查：

```bash
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo package -p flare-server-core
```

发布整个工作区时，先发布依赖 crate：

1. `flare-core-base`
2. `flare-core-runtime`
3. `flare-core-infra`
4. `flare-core-transport`
5. `flare-core-messaging`
6. `flare-server-core`

## 许可证

基于 [Apache License, Version 2.0](LICENSE) 授权。

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
