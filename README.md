# Flare Server Core

Flare IM Server Core Library - 为 Flare IM Server 提供完整的 gRPC 基础设施，包括拦截器、中间件、错误处理、服务发现等。

---

## 📚 功能模块

### 1. **错误处理** (`error`)

完整的错误处理系统，与 `flare-core` 完全适配，支持国际化：

#### 使用 FlareError（推荐）

```rust
use flare_server_core::{FlareError, ErrorCode, ErrorBuilder};

// 方式1：使用便捷方法
let err = FlareError::user_not_found("user123");
let err = FlareError::authentication_failed("Token invalid");
let err = FlareError::message_send_failed("Network error");

// 方式2：使用错误构建器
let err = ErrorBuilder::new(ErrorCode::MessageSendFailed, "消息发送失败")
    .param("message_id", "msg123")
    .param("user_id", "user456")
    .details("网络连接中断")
    .build_error();

// 方式3：直接创建
let err = FlareError::localized(ErrorCode::UserNotFound, "用户不存在");
```

#### 转换为 gRPC Status

```rust
use flare_server_core::FlareError;
use tonic::Status;

let flare_err = FlareError::user_not_found("user123");
let status: Status = flare_err.into(); // 自动转换
```

#### 国际化支持

```rust
use flare_server_core::{I18n, LocalizedError, default_zh_cn_translations};

let i18n = I18n::new("zh-CN");
i18n.load_translations("zh-CN", default_zh_cn_translations()).await;

let error = LocalizedError::new(ErrorCode::UserNotFound, "用户不存在");
let translated = i18n.translate_error(&error, Some("zh-CN")).await;
```

**错误代码分类**：
- **1000-1999**: 连接相关错误
- **2000-2999**: 认证相关错误
- **3000-3999**: 协议相关错误
- **4000-4999**: 消息相关错误
- **5000-5999**: 用户相关错误
- **6000-6999**: 系统相关错误
- **7000-7999**: 网络相关错误
- **8000-8999**: 序列化相关错误
- **9000-9999**: 通用错误

### 2. **国际化** (`i18n`)

支持从文件加载翻译：

```rust
use flare_server_core::I18n;

let i18n = I18n::new("zh-CN");

// 从 TOML 文件加载
i18n.load_from_file("zh-CN", "i18n/zh-CN.toml").await?;

// 从 JSON 文件加载
i18n.load_from_json("en-US", "i18n/en-US.json").await?;

// 从目录加载所有翻译文件
i18n.load_from_dir("i18n/").await?;
```

**翻译文件格式示例** (`i18n/zh-CN.toml`):

```toml
CONNECTION_FAILED = "连接失败"
USER_NOT_FOUND = "用户不存在: {user_id}"
MESSAGE_SEND_FAILED = "消息发送失败"
```

### 3. **服务注册发现** (`registry`)

支持多种服务注册发现后端，并提供负载均衡功能：

#### 基础使用

```rust
use flare_server_core::{create_registry, RegistryConfig, ServiceRegistryTrait};

let config = RegistryConfig {
    registry_type: "etcd".to_string(),  // 或 "consul", "mesh"
    endpoints: vec!["http://localhost:2379".to_string()],
    namespace: "flare".to_string(),
    ttl: 30,
};

let mut registry = create_registry(config).await?;
registry.register(service_info).await?;

// 获取所有服务实例
let services = registry.discover("gateway").await?;
println!("找到 {} 个网关实例", services.len());

// 获取所有服务类型
let service_types = registry.list_service_types().await?;

// 获取所有服务实例（所有类型）
let all_services = registry.list_all_services().await?;
```

#### 负载均衡

```rust
use flare_server_core::{ServiceSelector, LoadBalanceStrategy};

// 创建服务选择器
let selector = ServiceSelector::new(LoadBalanceStrategy::ConsistentHash);

// 选择服务实例（使用一致性哈希，确保同一用户路由到同一网关）
let gateway = selector.select_service(&gateways, Some("user123")).await;

// 选择服务地址
let address = selector.select_address(&gateways, Some("user123")).await;
```

**负载均衡策略**：
- `RoundRobin` - 轮询
- `Random` - 随机
- `ConsistentHash` - 一致性哈希（推荐用于网关选择）
- `LeastConnections` - 最少连接

#### 服务管理器（推荐）

```rust
use flare_server_core::{ServiceManager, LoadBalanceStrategy, create_registry, RegistryConfig};

// 创建服务管理器（带缓存和负载均衡）
let registry = create_registry(config).await?;
let manager = ServiceManager::new(registry, LoadBalanceStrategy::ConsistentHash);

// 获取所有网关实例
let gateways = manager.get_gateway_instances().await?;

// 选择网关（用于消息推送，使用一致性哈希确保同一用户路由到同一网关）
let gateway = manager.select_gateway(Some("user123")).await?;

// 获取服务实例（带缓存）
let service = manager.get_service_instance("signaling", None).await?;

// 刷新缓存
manager.refresh_cache(Some("gateway")).await?;
```

#### 多网关部署场景

```rust
// 推送消息时，需要确定发到哪个网关
use flare_server_core::{ServiceManager, LoadBalanceStrategy};

let manager = ServiceManager::new(registry, LoadBalanceStrategy::ConsistentHash);

// 方式1：使用用户ID进行一致性哈希路由（推荐）
let gateway = manager.select_gateway_by_user(&user_id).await?;
if let Some(gateway) = gateway {
    // 推送到选定的网关
    push_to_gateway(&gateway, message).await?;
}

// 方式2：获取所有网关实例，然后选择
let gateways = manager.get_gateway_instances().await?;
for gateway in gateways {
    // 可以广播到所有网关，或根据业务逻辑选择
    push_to_gateway(&gateway, message).await?;
}

// 方式3：获取所有网关地址
let addresses = manager.get_gateway_addresses().await?;
```

### 4. **认证 & Token 管理** (`auth`)

- `TokenService`：基于 HS256 的 JWT 工具，支持签发、校验、刷新、撤销单个令牌以及撤销用户所有令牌。
- `TokenStore` trait：抽象令牌存储策略；默认提供 `RedisTokenStore`，按 `flare:token:*` 命名空间维护活跃/撤销状态。
- 使用示例：

```rust
use std::sync::Arc;
use flare_server_core::{TokenService, RedisTokenStore};

let store = Arc::new(RedisTokenStore::new("redis://127.0.0.1/")?);
let token_service = TokenService::new("secret", "flare-im", 3600)
    .with_store(store);

let token = token_service.generate_token("user-1", Some("device-A"), None)?;
let refreshed = token_service.refresh_token(&token)?;
token_service.revoke_token(&refreshed)?;
token_service.revoke_user("user-1")?;
```

- `AuthInterceptor` / `CompositeInterceptor` 自动集成 `TokenService`，认证失败将返回 `unauthenticated`。

### 5. **拦截器** (`interceptor`)

#### 认证拦截器

```rust
use flare_server_core::AuthInterceptor;
use std::sync::Arc;
use flare_server_core::TokenService;

let token_service = Arc::new(TokenService::new("secret", "flare-im", 3600));
let interceptor = CompositeInterceptor::new()
    .with_auth(token_service)
    .with_logging()
    .with_tracing();
```

#### 追踪拦截器

自动注入 `trace_id` 和 `request_id`：

```rust
use flare_server_core::TracingInterceptor;

let tracing = TracingInterceptor::new();
```

#### 日志拦截器

```rust
use flare_server_core::LoggingInterceptor;

let logging = LoggingInterceptor::new();
```

#### 组合拦截器

```rust
use flare_server_core::interceptor::CompositeInterceptor;

let interceptor = CompositeInterceptor::new()
    .with_auth("secret_key".to_string());
```

### 6. **中间件** (`middleware`)

#### 超时中间件

```rust
use flare_server_core::middleware::TimeoutLayer;
use std::time::Duration;

let timeout_layer = TimeoutLayer::new(Duration::from_secs(30));
```

#### 限流中间件

```rust
use flare_server_core::middleware::RateLimitLayer;

let rate_limit = RateLimitLayer::new(1000); // 1000 requests
```

### 7. **客户端** (`client`)

#### 客户端构建器

```rust
use flare_server_core::client::ClientBuilder;
use std::time::Duration;

let client = ClientBuilder::new()
    .address("http://localhost:50051")
    .connect_timeout(Duration::from_secs(5))
    .timeout(Duration::from_secs(30))
    .max_retries(3)
    .build()
    .await?;
```

### 8. **服务端** (`server`)

#### 服务器构建器

```rust
use flare_server_core::server::ServerBuilder;

let server = ServerBuilder::new()
    .addr("127.0.0.1:50051".parse()?)
    .max_concurrent_streams(1000)
    .build()?;
```

### 9. **健康检查** (`health`)

```rust
use flare_server_core::HealthService;
use flare_server_core::health::HealthStatus;

let health = HealthService::new();
health.set_status("my_service", HealthStatus::Serving).await;
```

### 10. **指标收集** (`metrics`)

```rust
use flare_server_core::metrics::{MetricsCollector, MetricsInterceptor};

let collector = MetricsCollector::new();
let metrics = MetricsInterceptor::new(collector.clone());
```

### 11. **重试策略** (`retry`)

```rust
use flare_server_core::retry::ExponentialBackoffPolicy;
use std::time::Duration;

let policy = ExponentialBackoffPolicy::new(
    5,                              // 最大重试次数
    Duration::from_millis(100),     // 基础延迟
    Duration::from_secs(10),        // 最大延迟
);
```

### 12. **工具函数** (`utils`)

```rust
use flare_server_core::utils;
use tonic::Request;

// 提取元数据
let user_id = utils::extract_user_id(&req);
let trace_id = utils::extract_trace_id(&req);
```

---

## 📦 依赖

```toml
[dependencies]
flare-server-core = { path = "../flare-server-core" }
# 启用指标收集
# flare-server-core = { path = "../flare-server-core", features = ["metrics"] }
# 启用链路追踪
# flare-server-core = { path = "../flare-server-core", features = ["tracing"] }
```

---

## 🔧 与其他模块的关系

```
flare-server-core
├── 依赖 flare-proto (协议定义)
└── 被所有 gRPC 服务使用
    ├── flare-core-gateway
    ├── flare-signaling/*
    ├── flare-push/*
    ├── flare-storage/*
    └── flare-media
```

---

## 🎯 设计原则

1. **统一性**: 所有 gRPC 服务使用相同的基础设施
2. **可扩展性**: 易于添加自定义拦截器和中间件
3. **类型安全**: 充分利用 Rust 类型系统
4. **零开销抽象**: 不引入额外的运行时开销
5. **易用性**: 提供简洁的构建器 API
6. **多后端支持**: 支持多种服务注册发现后端
7. **负载均衡**: 支持多种负载均衡策略，特别适用于多网关部署

---

## 💡 最佳实践

### 多网关消息推送

```rust
use flare_server_core::{ServiceManager, LoadBalanceStrategy};

// 创建服务管理器，使用一致性哈希
let manager = ServiceManager::new(
    registry,
    LoadBalanceStrategy::ConsistentHash,
);

// 推送消息时，使用用户ID选择网关
let gateway = manager.select_gateway_by_user(&user_id).await?;
if let Some(gateway) = gateway {
    // 推送到选定的网关
    push_message_to_gateway(&gateway, message).await?;
}
```

### 服务发现缓存

```rust
// 服务管理器自动缓存服务列表，减少注册中心压力
// 默认缓存 TTL 为 30 秒，可以根据需要调整
manager.set_cache_ttl(Duration::from_secs(60));

// 手动刷新缓存
manager.refresh_cache(Some("gateway")).await?;
```

---

**维护者**: Flare IM Architecture Team  
**最后更新**: 2025-01-XX  
**版本**: 0.1.0
