# 微服务运行时框架

统一的服务生命周期管理框架，支持多种类型的任务（gRPC、消息消费者等），提供统一的服务注册、启动、关闭流程。

## 设计理念

1. **插件化任务系统**：通过 `Task` trait 支持不同类型的任务
2. **统一生命周期管理**：启动、就绪检查、服务注册、优雅关闭
3. **并发任务管理**：使用 `JoinSet` 管理所有后台任务
4. **优雅停机**：确保所有任务正确关闭，服务正确注销

## 核心组件

### Task Trait

所有需要在服务运行时执行的任务都需要实现 `Task` trait：

```rust
pub trait Task: Send {
    fn name(&self) -> &str;
    
    /// 获取任务依赖
    /// 返回此任务依赖的其他任务名称列表
    /// 依赖的任务会在此任务之前启动
    fn dependencies(&self) -> Vec<String> {
        Vec::new()  // 默认无依赖
    }
    
    fn run(
        self: Box<Self>,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>>;
    
    fn ready_check(&self) -> Pin<Box<dyn Future<Output = Result<...>> + Send + '_>>;
}
```

### ServiceRuntime

统一的服务运行时，管理所有任务的生命周期：

```rust
pub struct ServiceRuntime {
    service_name: String,
    service_address: SocketAddr,
    tasks: Vec<Box<dyn Task>>,
    registry: Option<ServiceRegistry>,
}
```

## 使用方式

### 1. 基本使用（不带服务注册）

适用于不需要服务注册的场景，或者服务注册在其他地方处理：

```rust
use flare_server_core::runtime::ServiceRuntime;
use flare_proto::session::session_service_server::SessionServiceServer;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address: SocketAddr = "127.0.0.1:50051".parse()?;
    let handler = SessionGrpcHandler::new(...);
    
    // 创建运行时并添加 gRPC 任务
    let runtime = ServiceRuntime::new("my-service", address)
        .add_grpc_task("grpc-server", address, move |builder| {
            builder.add_service(SessionServiceServer::new(handler))
        });
    
    // 运行服务（不带服务注册）
    runtime.run().await?;
    
    Ok(())
}
```

### 2. 带服务注册的使用（推荐）

使用 `run_with_registration` 方法，自动在任务就绪后注册服务。如果注册失败，服务会自动关闭：

```rust
use flare_server_core::runtime::ServiceRuntime;
use flare_proto::session::session_service_server::SessionServiceServer;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address: SocketAddr = "127.0.0.1:50051".parse()?;
    let handler = SessionGrpcHandler::new(...);
    
    // 创建运行时并运行（带服务注册）
    ServiceRuntime::new("session", address)
        .add_grpc_task("grpc-server", address, move |builder| {
            builder.add_service(SessionServiceServer::new(handler))
        })
        .run_with_registration(|addr| {
            Box::pin(async move {
                // 在任务就绪后自动注册服务
                // 如果注册失败，服务会自动关闭
                flare_im_core::discovery::register_service_only("session", addr, None).await
            })
        })
        .await?;
    
    Ok(())
}
```

**优势**：
- ✅ 自动在任务就绪后注册服务
- ✅ 如果注册失败，自动关闭服务并退出
- ✅ 代码简洁，只需一行调用

### 3. 添加多个任务

```rust
use flare_server_core::runtime::{ServiceRuntime, MessageConsumerTask};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address: SocketAddr = "127.0.0.1:50051".parse()?;
    
    // 创建运行时并添加多个任务
    let runtime = ServiceRuntime::new("my-service", address)
        // 添加 gRPC 任务
        .add_spawn_with_shutdown("grpc-server", |shutdown_rx| async move {
            // gRPC server code
            Ok(())
        })
        // 添加消息消费者任务
        .add_message_consumer(
            "kafka-consumer",
            Box::new(MyKafkaConsumer::new())
        );
    
    runtime.run().await?;
    
    Ok(())
}
```

### 4. 任务依赖管理

运行时支持任务依赖关系，确保依赖的任务在依赖它的任务之前启动。使用拓扑排序算法自动确定启动顺序。

#### 基本用法

```rust
use flare_server_core::runtime::ServiceRuntime;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address: SocketAddr = "127.0.0.1:50051".parse()?;
    
    let runtime = ServiceRuntime::new("my-service", address)
        // 1. 先启动数据库连接池（无依赖）
        .add_spawn("db-pool", async {
            // 初始化数据库连接池
            Ok(())
        })
        // 2. 然后启动缓存（依赖数据库）
        .add_spawn_with_deps(
            "cache",
            async {
                // 初始化缓存，需要数据库连接
                Ok(())
            },
            vec!["db-pool".to_string()]
        )
        // 3. 最后启动 gRPC 服务（依赖缓存）
        .add_spawn_with_shutdown_and_deps(
            "grpc-server",
            |shutdown_rx| async move {
                // gRPC server，需要缓存服务
                Ok(())
            },
            vec!["cache".to_string()]
        );
    
    // 任务会按依赖顺序启动：db-pool -> cache -> grpc-server
    runtime.run().await?;
    
    Ok(())
}
```

#### 消息消费者依赖

```rust
use flare_server_core::runtime::ServiceRuntime;

let runtime = ServiceRuntime::new("my-service", address)
    // 先启动 gRPC 服务
    .add_spawn_with_shutdown("grpc-server", |shutdown_rx| async move {
        Ok(())
    })
    // 然后启动消息消费者（依赖 gRPC 服务）
    .add_message_consumer_with_deps(
        "kafka-consumer",
        Box::new(MyKafkaConsumer::new()),
        vec!["grpc-server".to_string()]
    );
```

#### 依赖关系验证

运行时会自动验证：
- ✅ **循环依赖检测**：如果检测到循环依赖，会返回错误并列出涉及的任务
- ✅ **依赖存在性检查**：如果依赖的任务不存在，会返回错误并指出缺失的依赖

**错误示例**：

```rust
// ❌ 循环依赖：task-a 依赖 task-b，task-b 依赖 task-a
let runtime = ServiceRuntime::new("my-service", address)
    .add_spawn_with_deps("task-a", async { Ok(()) }, vec!["task-b".to_string()])
    .add_spawn_with_deps("task-b", async { Ok(()) }, vec!["task-a".to_string()]);

// 运行时会返回错误：
// Error: Circular dependency detected. Tasks involved: ["task-a", "task-b"]
```

```rust
// ❌ 不存在的依赖：task-b 依赖不存在的 task-c
let runtime = ServiceRuntime::new("my-service", address)
    .add_spawn_with_deps("task-b", async { Ok(()) }, vec!["task-c".to_string()]);

// 运行时会返回错误：
// Error: Task 'task-b' depends on 'task-c', but 'task-c' is not registered
```

#### 启动顺序日志

运行时会在日志中记录任务的启动顺序：

```
INFO Tasks sorted by dependencies: ["db-pool", "cache", "grpc-server"]
```

## 任务类型

### SpawnTask

封装任意 Future 的启动逻辑，支持依赖关系：

```rust
use flare_server_core::runtime::task::SpawnTask;

// 基本用法
let task = SpawnTask::new("my-task", async {
    // 任务逻辑
    Ok(())
});

// 带依赖的任务
let task = SpawnTask::new("my-task", async {
    Ok(())
})
.with_dependencies(vec!["other-task".to_string()]);

// 需要 shutdown 信号的任务
let task = SpawnTask::with_shutdown("my-task", |shutdown_rx| async move {
    // 监听 shutdown_rx
    Ok(())
});
```

### MessageConsumerTask

封装消息队列消费者的启动逻辑：

```rust
use flare_server_core::runtime::{MessageConsumerTask, MessageConsumer};

struct MyKafkaConsumer;

impl MessageConsumer for MyKafkaConsumer {
    fn consume(
        &self,
        shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send + '_>> {
        Box::pin(async move {
            // 实现消息消费逻辑
            // 监听 shutdown_rx 信号，收到后优雅退出
            loop {
                tokio::select! {
                    _ = shutdown_rx => {
                        break;
                    }
                    msg = kafka_consumer.poll() => {
                        // 处理消息
                    }
                }
            }
            Ok(())
        })
    }
}

// 基本用法
let task = MessageConsumerTask::new(
    "kafka-consumer",
    Box::new(MyKafkaConsumer)
);

// 带依赖的消息消费者
let task = MessageConsumerTask::new(
    "kafka-consumer",
    Box::new(MyKafkaConsumer)
)
.with_dependencies(vec!["grpc-server".to_string()]);
```

### 自定义任务

实现 `Task` trait 创建自定义任务：

```rust
use flare_server_core::runtime::Task;
use std::future::Future;
use std::pin::Pin;

struct MyCustomTask {
    name: String,
}

impl Task for MyCustomTask {
    fn name(&self) -> &str {
        &self.name
    }
    
    fn run(
        self: Box<Self>,
        mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send>> {
        Box::pin(async move {
            // 实现任务逻辑
            loop {
                tokio::select! {
                    _ = shutdown_rx => {
                        break;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        // 执行定时任务
                    }
                }
            }
            Ok(())
        })
    }
}
```

## 服务生命周期

`ServiceRuntime` 的执行流程：

1. **任务排序**：使用拓扑排序算法确定任务启动顺序（考虑依赖关系）
2. **启动所有任务**：按排序后的顺序使用 `JoinSet` 启动所有任务
3. **等待任务就绪**：通过 TCP 连接检查确保服务真正就绪
4. **注册服务**（如果配置了）：在所有任务就绪后注册到服务发现
5. **等待关闭信号**：监听 `Ctrl+C` 或注册失败信号
6. **优雅关闭**：发送关闭信号给所有任务，等待它们完成（最多 5 秒）
7. **注销服务**：如果配置了服务注册，在关闭时自动注销

## 最佳实践

1. **使用 `run_with_registration`**：如果需要在任务就绪后自动注册服务，使用此方法
2. **任务依赖管理**：合理使用任务依赖，确保基础设施（数据库、缓存等）先启动
3. **避免循环依赖**：设计任务依赖关系时避免循环依赖
4. **任务就绪检查**：实现 `ready_check` 方法确保任务真正就绪
5. **优雅关闭**：任务应该监听 `shutdown_rx` 信号，收到后优雅退出
6. **错误处理**：任务返回 `TaskResult`，运行时会自动记录错误

## 示例：重构现有服务

### 重构前（flare-session）

```rust
pub async fn run(&self) -> Result<()> {
    // 大量重复的启动、注册、关闭逻辑
    let mut join_set = JoinSet::new();
    // ... 100+ 行代码
}
```

### 重构后

```rust
pub async fn run(&self) -> Result<()> {
    use flare_server_core::runtime::ServiceRuntime;
    use flare_proto::session::session_service_server::SessionServiceServer;
    
    ServiceRuntime::new("session", self.address)
        .add_grpc_task("grpc-server", self.address, move |builder| {
            builder.add_service(SessionServiceServer::new(self.handler.clone()))
        })
        .run_with_registration(|addr| {
            Box::pin(async move {
                flare_im_core::discovery::register_service_only("session", addr, None).await
            })
        })
        .await
}
```

代码从 100+ 行减少到 10 行，且逻辑更清晰！

## 注意事项

1. **服务注册时机**：服务注册应该在所有任务就绪后、但服务开始接受请求前完成
2. **关闭信号**：任务应该尽快响应关闭信号，避免阻塞关闭流程
3. **错误处理**：如果服务注册失败，整个服务会关闭，确保服务不会在未注册的情况下运行
4. **任务启动顺序**：任务的启动顺序由依赖关系决定，使用拓扑排序算法自动确定
5. **循环依赖**：运行时会自动检测循环依赖并报错，设计时需避免
6. **依赖验证**：运行时会在启动前验证所有依赖的任务是否存在

