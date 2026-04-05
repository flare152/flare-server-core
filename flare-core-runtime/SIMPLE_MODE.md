# 简化模式使用指南

`ServiceRuntime` 提供了简化的运行模式,用于运行 MQ 消费者、自定义任务等简单场景,无需指定服务名和地址。

## 🚀 快速开始

### 1. 仅运行 MQ 消费者

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::mq_consumer()
        .add_spawn("kafka-consumer", async {
            // 消费 Kafka 消息
            println!("Consuming Kafka messages...");
            Ok(())
        })
        .add_spawn("nats-consumer", async {
            // 消费 NATS 消息
            println!("Consuming NATS messages...");
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

### 2. 仅运行自定义任务

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::tasks()
        .add_spawn("data-sync", async {
            // 数据同步任务
            println!("Syncing data...");
            Ok(())
        })
        .add_spawn("cache-refresh", async {
            // 缓存刷新任务
            println!("Refreshing cache...");
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

### 3. 使用 simple() 通用方法

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::simple()
        .add_spawn("task-1", async { Ok(()) })
        .add_spawn("task-2", async { Ok(()) })
        .run().await?;

    Ok(())
}
```

## 📋 API 对比

### 完整模式 (需要服务名和地址)

```rust
// 用于完整的微服务
ServiceRuntime::new("my-service")
    .with_address("0.0.0.0:8080".parse().unwrap())
    .add_spawn("grpc", async { Ok(()) })
    .run_with_registration(|addr| {
        Box::pin(async move {
            // 注册服务
            Ok(None)
        })
    }).await?;
```

### 简化模式 (无需服务名和地址)

```rust
// 用于 MQ 消费者、自定义任务
ServiceRuntime::mq_consumer()
    .add_spawn("kafka-consumer", async { Ok(()) })
    .run().await?;
```

## 🎯 使用场景

### 场景 1: MQ 消费者服务

```rust
use flare_core_runtime::ServiceRuntime;
use flare_core_messaging::mq::consumer::MqConsumerAdapterBuilder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Kafka 消费者
    let kafka_consumer = MqConsumerAdapterBuilder::new("kafka-consumer")
        .with_critical(false)
        .build(|| {
            async move {
                // 启动 Kafka 消费
                consume_kafka_messages().await
            }
        });

    // NATS 消费者
    let nats_consumer = MqConsumerAdapterBuilder::new("nats-consumer")
        .with_critical(false)
        .build(|| {
            async move {
                // 启动 NATS 消费
                consume_nats_messages().await
            }
        });

    ServiceRuntime::mq_consumer()
        .add_task(Box::new(kafka_consumer))
        .add_task(Box::new(nats_consumer))
        .run().await?;

    Ok(())
}
```

### 场景 2: 定时任务服务

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::tasks()
        .add_spawn("cleanup-job", async {
            // 定期清理任务
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                // 执行清理
            }
        })
        .add_spawn("metrics-export", async {
            // 定期导出指标
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                // 导出指标
            }
        })
        .run().await?;

    Ok(())
}
```

### 场景 3: 数据处理服务

```rust
use flare_core_runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ServiceRuntime::simple()
        .add_spawn("data-processor", async {
            // 处理数据
            Ok(())
        })
        .add_spawn("result-writer", async {
            // 写入结果
            Ok(())
        })
        .run().await?;

    Ok(())
}
```

## 🔧 高级用法

### 带依赖的任务

```rust
ServiceRuntime::tasks()
    .add_spawn("database", async {
        // 初始化数据库
        Ok(())
    })
    .add_spawn_with_deps(
        "data-processor",
        async {
            // 处理数据 (依赖数据库)
            Ok(())
        },
        vec!["database".to_string()]
    )
    .run().await?;
```

### 带 shutdown 的任务

```rust
use tokio::sync::oneshot;

ServiceRuntime::simple()
    .add_spawn_with_shutdown("long-running", |shutdown_rx| {
        async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        println!("Shutdown signal received");
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        // 执行任务
                    }
                }
            }
            Ok(())
        }
    })
    .run().await?;
```

### 状态监控

```rust
let runtime = ServiceRuntime::mq_consumer()
    .add_spawn("kafka-consumer", async { Ok(()) });

// 获取状态追踪器
let tracker = runtime.state_tracker();

// 订阅状态事件
let mut rx = tracker.subscribe();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        println!("Task {} state: {:?}", event.task_name, event.new_state);
    }
});

runtime.run().await?;
```

## 📊 对比总结

| 特性 | 完整模式 | 简化模式 |
|------|---------|---------|
| 服务名 | ✅ 必需 | ❌ 可选 (默认 "simple-runtime") |
| 服务地址 | ✅ 必需 (用于注册) | ❌ 不需要 |
| 服务注册 | ✅ 支持 | ❌ 不支持 |
| 任务管理 | ✅ 完整支持 | ✅ 完整支持 |
| 优雅停机 | ✅ 支持 | ✅ 支持 |
| 状态监控 | ✅ 支持 | ✅ 支持 |
| 依赖管理 | ✅ 支持 | ✅ 支持 |
| 适用场景 | 微服务 | MQ 消费者、定时任务 |

## 🎊 最佳实践

### 1. 选择合适的模式

- **完整模式**: HTTP/gRPC 微服务,需要服务发现
- **简化模式**: MQ 消费者、定时任务、数据处理

### 2. 任务命名

```rust
// ✅ 好的命名
ServiceRuntime::mq_consumer()
    .add_spawn("kafka-order-consumer", async { Ok(()) })
    .add_spawn("kafka-user-consumer", async { Ok(()) })

// ❌ 不好的命名
ServiceRuntime::mq_consumer()
    .add_spawn("consumer1", async { Ok(()) })
    .add_spawn("consumer2", async { Ok(()) })
```

### 3. 关键任务设置

```rust
ServiceRuntime::tasks()
    // 关键任务:失败会触发整体停机
    .add_spawn("critical-task", async { Ok(()) })
    // 非关键任务:失败不影响其他任务
    .add_spawn("non-critical-task", async { Ok(()) })
```

### 4. 优雅停机

```rust
ServiceRuntime::simple()
    .add_spawn_with_shutdown("graceful-task", |shutdown_rx| {
        async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        // 清理资源
                        println!("Cleaning up...");
                        break;
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        // 正常工作
                    }
                }
            }
            Ok(())
        }
    })
    .run().await?;
```

## 🎯 总结

简化模式提供了简洁的 API 用于运行 MQ 消费者、自定义任务等场景:

- ✅ 无需服务名和地址
- ✅ 完整的任务管理能力
- ✅ 支持依赖管理
- ✅ 支持优雅停机
- ✅ 支持状态监控
- ✅ 简洁的 API

选择合适的模式,让代码更简洁、更易维护!
