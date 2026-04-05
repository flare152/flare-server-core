# Flare Core Runtime - 统一运行时框架

[![Rust](https://img.shields.io/badge/rust-1.94.0%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**强大、稳定、通用**的 Rust 运行时框架，支持 10 亿+ 在线用户的高性能 IM 服务端。

## 🎯 核心特性

### 统一服务启动
- ✅ **HTTP 服务**: axum, volo-http, actix-web
- ✅ **gRPC 服务**: tonic, volo
- ✅ **MQ 消费者**: Kafka, RocketMQ
- ✅ **自定义任务**: 任意 async 任务
- ✅ **定时任务**: Cron 表达式调度

### 优雅停止
- ✅ **多信号源**: Ctrl+C, SIGTERM, SIGINT, 自定义通道
- ✅ **依赖顺序关闭**: 按依赖关系逆序关闭
- ✅ **超时强制终止**: 可配置超时时间

### 服务编排
- ✅ **任务依赖管理**: 拓扑排序、循环依赖检测
- ✅ **健康检查**: 定期检查、失败阈值
- ✅ **服务注册/注销**: Consul, Etcd, Nacos

### 状态监控
- ✅ **任务状态追踪**: 实时状态、事件通知
- ✅ **指标暴露**: Prometheus 格式
- ✅ **日志追踪**: 结构化日志

### 可扩展性
- ✅ **插件化架构**: 生命周期钩子
- ✅ **中间件链**: 任务执行前后插入逻辑
- ✅ **自定义适配器**: 支持未支持的框架

### 简化模式
- ✅ **MQ 消费者运行器**: 无需服务名和地址
- ✅ **自定义任务运行器**: 简洁的任务管理
- ✅ **灵活选择**: 完整模式 vs 简化模式

## 📦 架构设计

### 职责分离

`flare-core-runtime` **只提供规范**（trait 定义、配置、核心抽象），具体实现由其他 crate 提供：

- `flare-core-transport` - HTTP/gRPC 适配器实现
- `flare-core-messaging` - MQ 消费者适配器实现

### 核心抽象

| Trait | 说明 |
|-------|------|
| `Task` | 任务抽象，所有任务必须实现 |
| `ShutdownSignal` | 停机信号抽象 |
| `ServiceRegistry` | 服务注册抽象 |
| `HealthCheck` | 健康检查抽象 |
| `Plugin` | 插件抽象 |
| `Middleware` | 中间件抽象 |
| `MetricsCollector` | 指标收集抽象 |

## 🚀 快速开始

### 添加依赖

```toml
[dependencies]
flare-core-runtime = { version = "0.2", path = "path/to/flare-core-runtime" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

### 基础示例

```rust
use flare_core_runtime::ServiceRuntime;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 创建运行时
    let runtime = ServiceRuntime::new("my-service")
        .with_address("0.0.0.0:8080".parse().unwrap())
        .add_spawn("grpc-server", async {
            // 启动 gRPC 服务
            Ok(())
        })
        .add_spawn("kafka-consumer", async {
            // 启动 Kafka 消费者
            Ok(())
        });

    // 运行（等待 Ctrl+C）
    runtime.run().await?;

    Ok(())
}
```

### 简化模式示例

#### 仅运行 MQ 消费者

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::mq_consumer()
        .add_spawn("kafka-consumer", async {
            // 消费 Kafka 消息
            Ok(())
        })
        .add_spawn("nats-consumer", async {
            // 消费 NATS 消息
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

#### 仅运行自定义任务

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::tasks()
        .add_spawn("data-sync", async {
            // 数据同步任务
            Ok(())
        })
        .add_spawn("cache-refresh", async {
            // 缓存刷新任务
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

更多简化模式用法请参考 [SIMPLE_MODE.md](SIMPLE_MODE.md)。


### 带服务注册

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = ServiceRuntime::new("my-service")
        .with_address("0.0.0.0:8080".parse().unwrap())
        .add_spawn("grpc", async { Ok(()) });

    // 带服务注册
    runtime.run_with_registration(|addr| {
        Box::pin(async move {
            // 注册到 Consul/Etcd/Nacos
            println!("Registering service at {}", addr);
            Ok(None) // 返回注册器（可选）
        })
    }).await?;

    Ok(())
}
```

### 任务依赖

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = ServiceRuntime::new("my-service")
        // 先启动数据库
        .add_spawn("database", async {
            println!("Database started");
            Ok(())
        })
        // 再启动缓存（依赖数据库）
        .add_spawn_with_deps("cache", async {
            println!("Cache started");
            Ok(())
        }, vec!["database".to_string()])
        // 最后启动 gRPC（依赖缓存）
        .add_spawn_with_deps("grpc", async {
            println!("gRPC started");
            Ok(())
        }, vec!["cache".to_string()]);

    runtime.run().await?;
    Ok(())
}
```

### 自定义任务

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
            // 任务逻辑
            tokio::select! {
                _ = async {
                    // 主逻辑
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

## 📊 状态监控

### 订阅状态事件

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = ServiceRuntime::new("my-service")
        .add_spawn("task-1", async { Ok(()) });

    // 获取状态追踪器
    let tracker = runtime.state_tracker();

    // 订阅状态事件
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

## 🔧 配置

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

## 🎨 技术亮点

1. **Rust 2024 原生 async fn in traits** - 不使用 async-trait 宏
2. **零成本抽象** - 所有扩展点通过 trait 定义
3. **线程安全** - 使用 `Arc<RwLock>` 和 `broadcast` 通道
4. **事件驱动** - 状态变更自动发出事件
5. **错误处理规范** - 使用 `thiserror` 定义所有错误
6. **Builder 模式** - 所有配置提供 Builder 方法
7. **文档完整** - 所有类型都有文档注释和示例
8. **测试覆盖** - 所有核心组件都有单元测试

## 📚 API 文档

运行 `cargo doc --open` 查看完整的 API 文档。

## 🤝 贡献

欢迎贡献代码！请查看 [CONTRIBUTING.md](CONTRIBUTING.md) 了解详情。

## 📄 许可证

本项目采用 MIT 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🙏 致谢

感谢以下项目的启发：
- [Tokio](https://tokio.rs/) - 异步运行时
- [Tonic](https://github.com/hyperium/tonic) - gRPC 框架
- [Axum](https://docs.rs/axum/) - Web 框架
