# 统一运行时框架 - 集成示例

本文档展示如何使用 `flare-core-runtime` 管理 HTTP、gRPC 和 MQ 消费者服务。

## 📦 依赖配置

```toml
[dependencies]
flare-core-runtime = { path = "flare-core-runtime" }
flare-core-transport = { path = "flare-core-transport" }
flare-core-messaging = { path = "flare-core-messaging" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

## 🚀 完整示例

### 1. gRPC 服务

```rust
use flare_core_runtime::ServiceRuntime;
use flare_core_transport::grpc::GrpcAdapterBuilder;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let grpc_task = GrpcAdapterBuilder::new("grpc-service", "0.0.0.0:50051".parse().unwrap())
        .with_dependencies(vec!["database".to_string()])
        .build(|shutdown_rx| {
            async move {
                Server::builder()
                    .add_service(MyServiceServer::new(handler))
                    .serve_with_shutdown("0.0.0.0:50051".parse().unwrap(), async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .map_err(|e| e.into())
            }
        });

    ServiceRuntime::new("my-service")
        .add_task(Box::new(grpc_task))
        .run().await?;

    Ok(())
}
```

### 2. HTTP 服务

```rust
use flare_core_runtime::ServiceRuntime;
use flare_core_transport::http::HttpAdapterBuilder;
use axum::Router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let http_task = HttpAdapterBuilder::new("http-service", "0.0.0.0:8080".parse().unwrap())
        .with_dependencies(vec!["database".to_string()])
        .build(|shutdown_rx| {
            async move {
                let app = Router::new()
                    .route("/health", get(health_check));

                axum::Server::bind(&"0.0.0.0:8080".parse().unwrap())
                    .serve(app.into_make_service())
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .map_err(|e| e.into())
            }
        });

    ServiceRuntime::new("my-service")
        .add_task(Box::new(http_task))
        .run().await?;

    Ok(())
}
```

### 3. MQ 消费者

```rust
use flare_core_runtime::ServiceRuntime;
use flare_core_messaging::mq::consumer::MqConsumerAdapterBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let consumer_task = MqConsumerAdapterBuilder::new("kafka-consumer")
        .with_dependencies(vec!["database".to_string()])
        .with_critical(false) // 消费者失败不影响其他服务
        .build(|| {
            async move {
                // 启动 Kafka 消费者
                consumer.consume_messages().await
            }
        });

    ServiceRuntime::new("my-service")
        .add_task(Box::new(consumer_task))
        .run().await?;

    Ok(())
}
```

### 4. 完整微服务示例

```rust
use flare_core_runtime::ServiceRuntime;
use flare_core_transport::grpc::GrpcAdapterBuilder;
use flare_core_transport::http::HttpAdapterBuilder;
use flare_core_messaging::mq::consumer::MqConsumerAdapterBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. 数据库连接池任务
    let db_task = SpawnTask::new("database", async {
        // 初始化数据库连接池
        Ok(())
    });

    // 2. gRPC 服务 (依赖数据库)
    let grpc_task = GrpcAdapterBuilder::new("grpc", "0.0.0.0:50051".parse().unwrap())
        .with_dependencies(vec!["database".to_string()])
        .build(|shutdown_rx| {
            async move {
                Server::builder()
                    .add_service(MyServiceServer::new(handler))
                    .serve_with_shutdown("0.0.0.0:50051".parse().unwrap(), async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .map_err(|e| e.into())
            }
        });

    // 3. HTTP 服务 (依赖数据库)
    let http_task = HttpAdapterBuilder::new("http", "0.0.0.0:8080".parse().unwrap())
        .with_dependencies(vec!["database".to_string()])
        .build(|shutdown_rx| {
            async move {
                let app = Router::new().route("/health", get(health_check));
                axum::Server::bind(&"0.0.0.0:8080".parse().unwrap())
                    .serve(app.into_make_service())
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.await;
                    })
                    .await
                    .map_err(|e| e.into())
            }
        });

    // 4. Kafka 消费者 (依赖数据库)
    let consumer_task = MqConsumerAdapterBuilder::new("kafka-consumer")
        .with_dependencies(vec!["database".to_string()])
        .with_critical(false)
        .build(|| {
            async move {
                consumer.consume_messages().await
            }
        });

    // 5. 启动所有服务
    ServiceRuntime::new("my-microservice")
        .with_address("0.0.0.0:50051".parse().unwrap())
        .add_task(Box::new(db_task))
        .add_task(Box::new(grpc_task))
        .add_task(Box::new(http_task))
        .add_task(Box::new(consumer_task))
        .run_with_registration(|addr| {
            Box::pin(async move {
                // 注册到服务发现
                println!("Registering service at {}", addr);
                Ok(None)
            })
        }).await?;

    Ok(())
}
```

## 🎯 关键特性

### 依赖管理

服务按依赖顺序启动:
```
database -> grpc
         -> http
         -> kafka-consumer
```

### 优雅停机

- Ctrl+C 或 SIGTERM 触发停机
- 按依赖逆序关闭: `kafka-consumer -> http -> grpc -> database`
- 超时强制终止

### 服务注册

- 所有任务就绪后注册服务
- 停机前自动注销

### 状态监控

```rust
let runtime = ServiceRuntime::new("my-service");
let tracker = runtime.state_tracker();

// 订阅状态事件
let mut rx = tracker.subscribe();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        println!("Task {} state: {:?} -> {:?}",
            event.task_name, event.old_state, event.new_state);
    }
});

runtime.run().await?;
```

## 📊 架构优势

1. **统一管理**: 所有服务类型统一管理
2. **依赖管理**: 自动处理启动顺序
3. **优雅停机**: 按依赖逆序关闭
4. **可观测性**: 实时状态追踪
5. **可扩展**: 支持插件和中间件

## 🔧 高级用法

### 使用插件

```rust
use flare_core_runtime::plugin::{PluginManager, Plugin};

let mut plugin_manager = PluginManager::new();
plugin_manager.register(Arc::new(LoggingPlugin));

// 在运行时中调用
plugin_manager.on_startup(&ctx).await?;
```

### 使用中间件

```rust
use flare_core_runtime::middleware::{MiddlewareChain, Middleware};

let mut middleware_chain = MiddlewareChain::new();
middleware_chain.add(Arc::new(LoggingMiddleware));

// 在任务执行前后调用
middleware_chain.before("task-1").await?;
// 执行任务
middleware_chain.after("task-1", &result).await?;
```

### 使用健康检查

```rust
use flare_core_runtime::health::{HealthChecker, HealthCheck};

let mut health_checker = HealthChecker::new()
    .with_failure_threshold(3);

health_checker.add_check(Arc::new(DatabaseHealthCheck));

// 定期检查
let is_healthy = health_checker.is_healthy().await;
```

## 🎊 总结

通过 `flare-core-runtime` 提供的统一抽象,可以轻松管理各种类型的服务:

- ✅ HTTP 服务 (axum, actix-web)
- ✅ gRPC 服务 (tonic)
- ✅ MQ 消费者 (Kafka, NATS)
- ✅ 自定义任务
- ✅ 定时任务

所有服务统一管理,自动处理依赖、优雅停机、状态监控等复杂逻辑。
