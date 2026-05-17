# flare-core-base

`flare-core-base` 是 `flare-server-core` 的基础能力库，提供跨服务可复用的公共类型与基础设施能力。

## 主要职责

- **Context**：请求上下文与链路透传（`trace_id` / `request_id` / `tenant_id` / `user_id` 等）。
- **Error**：统一错误模型、错误码、构建器与 gRPC 友好转换。
- **Config**：基础配置结构与分层配置读取能力。
- **I18n**：默认中英文消息与国际化支持。
- **Types/Utils**：公共类型与通用工具函数。

## 模块结构

- `src/context`：上下文对象、类型安全扩展字段、辅助工具。
- `src/error`：`FlareError`、`ErrorCode`、错误构建器与转换。
- `src/config`：配置结构体与分层读取器。
- `src/i18n`：默认翻译与语言资源接口。
- `src/types`：服务基础类型定义。
- `src/utils`：跨模块工具函数。

## 配置方案（Env 优先，TOML 兜底）

当前提供 `config::LayeredConfig`：

- 优先读取环境变量；
- 环境变量缺失时读取 TOML；
- 适合跨服务统一配置注入机制（不绑定具体业务配置结构体）。

示例：

```rust
use std::path::Path;
use flare_core_base::config::LayeredConfig;

let layered = LayeredConfig::from_optional_toml(Some(Path::new("config/services/capability.toml")));

let timeout_ms = layered
    .resolve_u64("FLARE_CAPABILITY_PLUGIN_CALL_TIMEOUT_MS", "capability_runtime.plugin_call_timeout_ms")
    .unwrap_or(5000);
```

## 设计原则

- **机制下沉，业务上浮**：只抽通用机制，不承载业务模型。
- **显式优先级**：环境变量覆盖 TOML，行为可预测。
- **零侵入复用**：通过泛型/基础 API 供各服务按需接入。
- **稳定接口**：优先保持跨 crate 的向后稳定性。

## 适用场景

- 多服务共享一套配置加载规则；
- 统一错误与上下文模型；
- 降低各服务重复实现基础逻辑的成本。
