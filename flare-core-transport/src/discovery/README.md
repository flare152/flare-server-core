# 统一服务发现与负载均衡模块

## 概述

本模块提供统一的服务发现抽象，支持多种后端（etcd、consul、DNS、Service Mesh），完全兼容 tower 生态系统，支持命名空间、版本控制和自定义标签。

## 特性

- ✅ **多后端支持**: etcd、consul、DNS/SRV、Service Mesh (xDS)
- ✅ **Tower 兼容**: 实现 `tower::discover::Discover` trait，可与 `tower::balance` 无缝集成
- ✅ **配置驱动**: 通过配置切换服务发现方式，业务代码无感知
- ✅ **服务注册**: 支持服务实例注册和注销
- ✅ **Channel 池**: 每个 Endpoint 对应一个缓存的 Channel，自动复用
- ✅ **重试机制**: 集成 tower::retry 中间件，支持自动重试
- ✅ **健康检查**: 自动检查服务健康状态，失败节点暂时剔除
- ✅ **异步并发**: 支持异步并发批量调用
- ✅ **命名空间支持**: 支持多命名空间隔离
- ✅ **版本控制**: 支持服务版本管理和路由
- ✅ **自定义标签**: 支持丰富的标签过滤和路由
- ✅ **负载均衡**: 支持多种负载均衡策略（轮询、随机、一致性哈希、最少连接、加权等）

## 架构设计

```
┌─────────────────────────────────────────────────────────┐
│                 业务服务层 (无感知)                        │
│             使用标准 tonic + tower 客户端                  │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│              ServiceDiscover (tower::discover::Discover)   │
│  ┌────────────────────────────────────────────────────┐  │
│  │  LoadBalancer (tower::balance::Balance)           │  │
│  └────────────────────────────────────────────────────┘  │
└────────────────────┬────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────┐
│              DiscoveryBackend (Trait)                    │
│  ┌──────────┬──────────┬──────────┬──────────┐         │
│  │  Etcd    │  Consul  │   DNS    │   Mesh   │         │
│  └──────────┴──────────┴──────────┴──────────┘         │
└─────────────────────────────────────────────────────────┘
```

## 使用方式

### 0. 快速构建（推荐用于生产环境）

使用 `register_and_discover` 方法，一键完成服务注册和发现，自动处理心跳续期和生命周期管理：

```rust
use flare_server_core::discovery::{
    DiscoveryFactory, BackendType, ServiceInstance, InstanceMetadata,
};
use std::collections::HashMap;
use std::net::SocketAddr;

// 创建服务实例
let instance = ServiceInstance {
    service_type: "signaling-online".to_string(),
    instance_id: format!("node-{}", uuid::Uuid::new_v4()),
    address: "127.0.0.1:8080".parse::<SocketAddr>()?,
    namespace: Some("production".to_string()),
    version: Some("v1.0.0".to_string()),
    tags: {
        let mut tags = HashMap::new();
        tags.insert("region".to_string(), "us-east-1".to_string());
        tags
    },
    metadata: InstanceMetadata {
        region: Some("us-east-1".to_string()),
        ..Default::default()
    },
    healthy: true,
    weight: 100,
};

// 快速构建：服务注册 + 发现（使用最优默认配置）
let (registry, discover, _updater) = DiscoveryFactory::register_and_discover(
    BackendType::Etcd,
    vec!["http://localhost:2379".to_string()],
    "signaling-online".to_string(),
    instance,
).await?;

// registry 会自动处理心跳续期（每 30 秒）
// 当 registry 被 drop 时，会自动注销服务

// 使用 discover 进行服务发现
use flare_server_core::discovery::ServiceClient;
let mut client = ServiceClient::new(discover);
let channel = client.get_channel().await?;

// 创建 gRPC 客户端并调用
// let mut grpc_client = YourServiceClient::new(channel);
// let response = grpc_client.your_method(request).await?;

// 保持 registry 存活（在实际应用中，应该保持到服务关闭）
let _registry = registry;
```

**默认配置（最优实践）**：
- ✅ 心跳间隔：30 秒（平衡网络开销和故障检测速度）
- ✅ TTL：90 秒（心跳间隔的 3 倍，确保网络抖动时不会误判）
- ✅ 刷新间隔：30 秒（与服务发现同步）
- ✅ 健康检查：启用，间隔 10 秒，超时 5 秒
- ✅ 负载均衡：一致性哈希（适合大多数场景）
- ✅ 失败阈值：3 次
- ✅ 成功阈值：2 次

### 1. 服务注册

```rust
use flare_server_core::discovery::{
    DiscoveryConfig, DiscoveryFactory, BackendType, ServiceInstance, InstanceMetadata,
};
use std::collections::HashMap;
use std::net::SocketAddr;

// 创建后端配置
let mut backend_config = HashMap::new();
backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));

let config = DiscoveryConfig {
    backend: BackendType::Etcd,
    backend_config,
    // ... 其他配置
};

// 创建后端
let backend = DiscoveryFactory::create_backend(&config).await?;

// 注册服务实例
let instance = ServiceInstance {
    service_type: "signaling-online".to_string(),
    instance_id: "node-1".to_string(),
    address: "127.0.0.1:8080".parse::<SocketAddr>()?,
    namespace: Some("production".to_string()),
    version: Some("v1.0.0".to_string()),
    tags: {
        let mut tags = HashMap::new();
        tags.insert("region".to_string(), "us-east-1".to_string());
        tags
    },
    metadata: InstanceMetadata {
        region: Some("us-east-1".to_string()),
        ..Default::default()
    },
    healthy: true,
    weight: 100,
};

// 注册服务
backend.register(instance.clone()).await?;

// 注销服务（服务关闭时）
backend.unregister(&instance.instance_id).await?;
```

### 2. 使用默认配置快速创建服务发现

如果只需要服务发现（不需要注册），可以使用 `create_with_defaults`：

```rust
use flare_server_core::discovery::{DiscoveryFactory, BackendType};

// 使用最优默认配置创建服务发现
let (discover, _updater) = DiscoveryFactory::create_with_defaults(
    BackendType::Etcd,
    vec!["http://localhost:2379".to_string()],
    "signaling-online".to_string(),
).await?;

// 使用 ServiceClient 进行服务发现
use flare_server_core::discovery::ServiceClient;
let mut client = ServiceClient::new(discover);
let channel = client.get_channel().await?;
```

### 3. 服务发现（配置驱动方式，推荐）

```rust
use flare_server_core::discovery::{
    DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy, ServiceClient,
};
use std::collections::HashMap;
use serde_json::json;

// 创建配置
let mut backend_config = HashMap::new();
backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
backend_config.insert("service_type".to_string(), json!("signaling-online"));

let config = DiscoveryConfig {
    backend: BackendType::Etcd,
    backend_config,
    namespace: None,
    version: None,
    tag_filters: vec![],
    load_balance: LoadBalanceStrategy::ConsistentHash,
    health_check: Some(HealthCheckConfig {
        interval: 10,
        timeout: 5,
        failure_threshold: 3,
        success_threshold: 2,
        path: Some("/health".to_string()),
    }),
    refresh_interval: Some(30),
};

// 创建服务发现器（带自动刷新）
let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;

// 创建服务客户端（集成 Channel 缓存和 P2C 负载均衡）
let mut client = ServiceClient::new(discover);

// 简洁调用：自动完成服务发现 + 负载均衡 + Channel 获取
let channel = client.get_channel().await?;

// 创建 gRPC 客户端并调用
// let mut grpc_client = YourServiceClient::new(channel);
// let response = grpc_client.your_method(request).await?;
```

### 3. Channel 池 + 重试

```rust
use tower::retry::RetryLayer;
use tower::ServiceBuilder;
use std::time::Duration;

// 创建服务客户端
let mut client = ServiceClient::new(discover);

// 获取 Channel（自动从池中复用）
let channel = client.get_channel().await?;

// 创建 gRPC 客户端
let mut grpc_client = YourServiceClient::new(channel);

// 使用重试策略
let retry_policy = RetryLayer::new(
    tower::retry::Retry::new(
        tower::retry::policy::ConstantBackoff::new(Duration::from_millis(100))
            .max_retries(3),
    ),
);

let mut client_with_retry = ServiceBuilder::new()
    .layer(retry_policy)
    .service(grpc_client);

// 调用（自动重试）
let response = client_with_retry.call(request).await?;
```

### 4. 健康检查（失败节点暂时剔除）

```rust
let config = DiscoveryConfig {
    // ...
    health_check: Some(HealthCheckConfig {
        interval: 10,  // 每 10 秒检查一次
        timeout: 5,    // 超时 5 秒
        failure_threshold: 3,  // 连续失败 3 次后标记为不健康
        success_threshold: 2,  // 连续成功 2 次后标记为健康
        path: Some("/health".to_string()),
    }),
    // ...
};

// 健康检查会自动运行，失败的节点会被暂时剔除
// 当节点恢复健康后，会自动重新加入负载均衡池
let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
let mut client = ServiceClient::new(discover);

// 获取 Channel（自动跳过不健康的节点）
let channel = client.get_channel().await?;
```

### 5. 异步并发批量调用

```rust
use std::sync::Arc;
use tokio::sync::Mutex;
use futures::future;

let client = Arc::new(Mutex::new(ServiceClient::new(discover)));

// 方式 1: 使用 futures::future::join_all 并发调用
let results = future::join_all((0..10).map(|i| {
    let client = client.clone();
    async move {
        let channel = {
            let mut client = client.lock().await;
            client.get_channel().await?
        };
        // 创建 gRPC 客户端并调用
        // let mut grpc_client = YourServiceClient::new(channel);
        // let response = grpc_client.your_method(request).await?;
        Ok(())
    }
})).await;

// 方式 2: 使用 tokio::spawn 并发调用
let handles: Vec<_> = (0..10).map(|i| {
    let client = client.clone();
    tokio::spawn(async move {
        let channel = {
            let mut client = client.lock().await;
            client.get_channel().await?
        };
        // 创建 gRPC 客户端并调用
        // ...
        Ok(())
    })
}).collect();

let results = future::join_all(handles).await;
```

### 7. 命名空间和版本控制

```rust
use flare_server_core::discovery::{
    DiscoveryConfig, NamespaceConfig, VersionConfig,
};

let config = DiscoveryConfig {
    backend: BackendType::Etcd,
    backend_config: /* ... */,
    namespace: Some(NamespaceConfig {
        default: Some("production".to_string()),
        separator: Some("/".to_string()),
    }),
    version: Some(VersionConfig {
        default: Some("v1.0.0".to_string()),
        format: Some("semver".to_string()),
        enable_routing: true,
    }),
    // ...
};
```

### 8. 自定义标签过滤

```rust
use flare_server_core::discovery::TagFilter;

let config = DiscoveryConfig {
    // ...
    tag_filters: vec![
        TagFilter {
            key: "environment".to_string(),
            value: Some("production".to_string()),
            pattern: Some("exact".to_string()),
        },
        TagFilter {
            key: "region".to_string(),
            value: Some("us-east-1".to_string()),
            pattern: Some("exact".to_string()),
        },
    ],
    // ...
};
```

## 配置示例

### etcd 配置

```toml
[discovery]
backend = "etcd"
refresh_interval = 30
load_balance = "consistent_hash"

[discovery.backend_config]
endpoints = ["http://localhost:2379"]
service_type = "signaling-online"
ttl = 30

[discovery.namespace]
default = "production"
separator = "/"

[discovery.version]
default = "v1.0.0"
format = "semver"
enable_routing = true
```

### consul 配置

```toml
[discovery]
backend = "consul"
refresh_interval = 30
load_balance = "round_robin"

[discovery.backend_config]
url = "http://localhost:8500"
service_type = "message-orchestrator"
```

### DNS 配置

```toml
[discovery]
backend = "dns"
refresh_interval = 60
load_balance = "random"

[discovery.backend_config]
domain = "local"
addresses = ["127.0.0.1:8080", "127.0.0.1:8081"]
service_type = "storage-reader"
```

### Service Mesh 配置

```toml
[discovery]
backend = "mesh"
refresh_interval = 30
load_balance = "least_connections"

[discovery.backend_config]
xds_server = "http://localhost:8080"
service_type = "session"
```

## 负载均衡策略

- `round_robin`: 轮询
- `random`: 随机
- `consistent_hash`: 一致性哈希（推荐用于需要会话粘性的场景）
- `least_connections`: 最少连接
- `weighted_round_robin`: 加权轮询
- `weighted_random`: 加权随机

## 快速构建方法

### ServiceRegistry（服务注册器）

`ServiceRegistry` 自动处理服务注册、心跳续期和注销：

```rust
use flare_server_core::discovery::{DiscoveryFactory, ServiceRegistry, BackendType, ServiceInstance};

// 创建服务实例
let instance = ServiceInstance::new(
    "signaling-online",
    "node-1",
    "127.0.0.1:8080".parse()?,
);

// 创建后端并注册
let backend = DiscoveryFactory::create_backend(&config).await?;
backend.register(instance.clone()).await?;

// 创建注册器（自动处理心跳）
let registry = ServiceRegistry::new(backend, instance, 30); // 30 秒心跳间隔

// 手动发送心跳（通常不需要）
registry.heartbeat().await?;

// 更新服务实例信息
let mut updated_instance = registry.instance().clone();
updated_instance.weight = 200;
registry.update_instance(updated_instance).await?;

// 当 registry 被 drop 时，会自动注销服务
```

**特性**：
- ✅ 自动心跳续期（每 30 秒）
- ✅ 优雅关闭（自动注销）
- ✅ 支持动态更新实例信息
- ✅ 错误重试和日志记录

## 核心功能

### Channel 池

每个 Endpoint 对应一个缓存的 `tonic::Channel`，节点上线后立即创建，调用直接复用。Channel 在节点下线时自动清理。

### P2C 负载均衡

使用 `tower::balance::p2c::Balance` 进行负载均衡，随机选择两个节点，选择负载较低的一个。可简单替换为更复杂策略。

### 动态服务发现

通过 etcd watch 或定时刷新处理节点上线/下线，Channel 自动更新。支持实时服务发现。

### 健康检查

自动检查服务健康状态：
- 定期检查服务健康端点（如 `/health`）
- 连续失败达到阈值后标记为不健康，暂时剔除
- 连续成功达到阈值后重新加入负载均衡池
- 自动跳过不健康的节点

### 重试机制

集成 `tower::retry` 中间件，支持：
- 指数退避重试
- 最大重试次数限制
- 可配置的重试策略

### 异步并发批量调用

支持异步并发批量调用：
- 使用 `futures::future::join_all` 并发调用
- 使用 `tokio::spawn` 并发调用
- 自动负载均衡和 Channel 复用

## 最佳实践

1. **生产环境**: 使用 etcd 或 consul，启用健康检查和版本控制
2. **开发环境**: 可以使用 DNS 或直接地址配置
3. **大规模集群**: 使用 Service Mesh (xDS) 模式
4. **会话粘性**: 使用一致性哈希策略
5. **高可用**: 配置多个后端节点和健康检查
6. **Channel 复用**: 使用 `ServiceClient` 自动管理 Channel 池
7. **重试策略**: 为关键调用添加重试机制
8. **并发调用**: 使用异步并发批量调用提高性能

## 注意事项

- DNS 后端不支持服务注册（只读）
- Service Mesh 模式下，服务注册由 sidecar 处理
- 一致性哈希需要提供 key（如 user_id）才能生效
- 健康检查需要服务提供 `/health` 端点

