# Context 系统使用指南

## 概述

`flare-server-core` 提供了一个类型安全、显式传递的上下文系统，支持取消、超时和自定义数据存储。

## 核心特性

### 1. 类型安全
- ✅ 所有元数据都是显式字段，编译时检查
- ✅ 支持自定义数据存储（使用 `TypeMap`，类型安全）
- ✅ 可以存储任意 `Send + Sync` 类型的数据（包括数据库连接、服务实例等）

### 2. 显式传递
- ✅ Context 作为函数参数传递，不使用隐式全局变量
- ✅ 不使用 TLS，所有 Context 必须显式传递

### 3. 取消语义
- ✅ 基于 `tokio::sync::watch::channel` 实现
- ✅ 支持 `cancel()`、`is_cancelled()`、`cancelled().await`
- ✅ 父子 Context 取消传播

### 4. 超时支持
- ✅ `with_timeout(Duration)` - 创建带超时的子 Context
- ✅ `with_deadline(Instant)` - 创建带截止时间的子 Context
- ✅ `remaining_time()` - 获取剩余时间
- ✅ 自动选择更早的截止时间（父子 Context）

### 5. 父子关系
- ✅ `child()` - 创建子 Context
- ✅ 父 Context 取消 → 子 Context 自动取消
- ✅ 子 Context 取消不影响父 Context

### 6. 上下文兼容
- ✅ 兼容 `TenantContext`、`RequestContext`、`TraceContext`、`ActorContext`、`DeviceContext`、`AuditContext`
- ✅ 支持从现有中间件（`RequestContextLayer`、`TenantLayer`）自动提取
- ✅ 支持自动构建 gRPC 请求 metadata

## 核心 API

### Context 创建

```rust
use flare_server_core::context::{Context, TenantContext, RequestContext};
use std::time::Duration;

// 创建根 Context
let ctx = Context::root();

// 创建带请求 ID 的根 Context
let ctx = Context::with_request_id("req-123");

// 链式调用设置元数据
let ctx = Context::with_request_id("req-123")
    .with_trace_id("trace-456")
    .with_user_id("user-789")
    .with_tenant_id("tenant-abc");

// 设置上下文对象
let ctx = Context::with_request_id("req-123")
    .with_tenant(TenantContext::new("tenant-456"))
    .with_request(RequestContext::new("req-123"));
```

### 自定义数据存储

```rust
use flare_server_core::context::Context;
use std::sync::Arc;
use sqlx::PgPool;

// 存储数据库连接
let db_pool: Arc<PgPool> = get_db_pool();
let ctx = ctx.insert_data(db_pool);

// 存储服务实例
struct MyService;
let service = Arc::new(MyService);
let ctx = ctx.insert_data(service);

// 获取自定义数据
let pool: Option<&Arc<PgPool>> = ctx.get_data();
let service: Option<&Arc<MyService>> = ctx.get_data();
```

### 创建子 Context

```rust
// 创建子 Context（继承父 Context 的取消和超时）
let child = parent.child();

// 创建带超时的子 Context
let timeout_ctx = parent.with_timeout(Duration::from_secs(5));

// 创建带截止时间的子 Context
let deadline = Instant::now() + Duration::from_secs(10);
let deadline_ctx = parent.with_deadline(deadline);
```

### 取消和检查

```rust
// 取消 Context
ctx.cancel();

// 检查是否已取消
if ctx.is_cancelled() {
    return Err(anyhow::anyhow!("request cancelled"));
}

// 等待取消信号
ctx.cancelled().await;
```

### 在异步函数中使用

```rust
use flare_server_core::context::{Context, ContextExt};
use tokio::select;

async fn process_request(ctx: &Context) -> Result<String> {
    // 检查取消状态
    ctx.ensure_not_cancelled()?;
    
    // 获取自定义数据
    let pool: Option<&Arc<PgPool>> = ctx.get_data();
    
    // 获取上下文
    let tenant = ctx.tenant();
    let request = ctx.request();
    
    // 在 select! 中使用
    select! {
        result = do_work() => result,
        _ = ctx.cancelled() => {
            Err(anyhow::anyhow!("cancelled"))
        }
    }
}
```

## 与现有系统集成

### 在 gRPC 客户端中使用

```rust
use flare_server_core::client::{set_context_metadata, request_with_context};
use flare_server_core::context::{Context, TenantContext, RequestContext};
use tonic::Request;

// 方式 1: 手动设置
let ctx = Context::with_request_id("req-123")
    .with_tenant(TenantContext::new("tenant-456"))
    .with_request(RequestContext::new("req-123"));

let mut request = Request::new(my_payload);
set_context_metadata(&mut request, &ctx);

// 方式 2: 便捷函数（自动构建 metadata）
let request = request_with_context(my_payload, &ctx);
```

### 在 gRPC 服务端中使用

```rust
use flare_server_core::middleware::{ContextLayer, extract_context};
use flare_server_core::context::Context;
use tonic::{Request, Response, Status};

// 1. 在 bootstrap 中应用中间件
// ContextLayer 会自动从 RequestContext 和 TenantContext 中间件提取信息
Server::builder()
    .layer(TenantLayer::new().allow_missing())
    .layer(RequestContextLayer::new().allow_missing())
    .layer(ContextLayer::new().allow_missing()) // 放在最后，可以提取前面的中间件信息
    .add_service(MyServiceServer::new(handler))
    .serve(addr)
    .await?;

// 2. 在 handler 中提取 Context
async fn my_handler(
    &self,
    request: Request<MyRequest>,
) -> Result<Response<MyResponse>, Status> {
    let ctx = extract_context(&request)?;
    
    // 使用 Context
    if ctx.is_cancelled() {
        return Err(Status::cancelled("request cancelled"));
    }
    
    // 获取上下文
    let tenant = ctx.tenant();
    let request_ctx = ctx.request();
    
    // 获取自定义数据
    let db_pool: Option<&Arc<PgPool>> = ctx.get_data();
    
    // 创建子 Context 用于下游调用
    let child_ctx = ctx.child().with_timeout(Duration::from_secs(5));
    
    // 调用下游服务
    let result = call_downstream(&child_ctx).await?;
    
    Ok(Response::new(MyResponse { result }))
}
```

### 上下文类型转换

```rust
use flare_server_core::context::{Context, RequestContext, TenantContext};

// 从 RequestContext 创建 Context
let req_ctx = RequestContext::new("req-123")
    .with_actor(ActorContext::new("user-456"));
let ctx = Context::from(&req_ctx);

// 从 TenantContext 创建 Context
let tenant = TenantContext::new("tenant-123");
let ctx = Context::from(&tenant);

// 从 Context 创建 RequestContext（自动构建）
let req_ctx: RequestContext = (&ctx).into();

// 从 Context 创建 TenantContext（自动构建）
let tenant: TenantContext = (&ctx).into();
```

## 完整示例

### 示例 1: IM 消息发送（带数据库连接）

```rust
use flare_server_core::context::{Context, ContextExt};
use flare_server_core::context::{TenantContext, RequestContext};
use std::sync::Arc;
use sqlx::PgPool;
use std::time::Duration;

pub struct MessageService {
    db_pool: Arc<PgPool>,
}

impl MessageService {
    pub async fn send_message(
        &self,
        ctx: &Context,
        conversation_id: &str,
        content: &str,
    ) -> Result<String> {
        // 检查取消状态
        ctx.ensure_not_cancelled()?;
        
        // 获取数据库连接（优先从 Context 获取，否则使用默认）
        let pool = ctx.get_data::<Arc<PgPool>>()
            .unwrap_or(&self.db_pool);
        
        // 获取租户上下文
        let tenant = ctx.tenant()
            .ok_or_else(|| anyhow::anyhow!("tenant context required"))?;
        
        // 创建子 Context 用于路由（5秒超时）
        let route_ctx = ctx.child().with_timeout(Duration::from_secs(5));
        
        // 路由消息
        let gateway = self.route_message(&route_ctx, conversation_id).await?;
        
        // 创建子 Context 用于发送（10秒超时）
        let send_ctx = ctx.child().with_timeout(Duration::from_secs(10));
        
        // 发送到网关
        let message_id = self.send_to_gateway(&send_ctx, &gateway, content).await?;
        
        Ok(message_id)
    }
    
    async fn route_message(&self, ctx: &Context, conversation_id: &str) -> Result<String> {
        ctx.check_cancelled()?;
        
        tokio::select! {
            result = async {
                // 模拟路由查找
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(format!("gateway-{}", conversation_id))
            } => result,
            _ = ctx.cancelled() => {
                Err(anyhow::anyhow!("routing cancelled"))
            }
        }
    }
    
    async fn send_to_gateway(
        &self,
        ctx: &Context,
        gateway: &str,
        content: &str,
    ) -> Result<String> {
        ctx.check_cancelled()?;
        
        tokio::select! {
            result = async {
                // 模拟网络请求
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(format!("msg-{}", uuid::Uuid::new_v4()))
            } => result,
            _ = ctx.cancelled() => {
                Err(anyhow::anyhow!("send cancelled"))
            }
        }
    }
}
```

### 示例 2: 取消传播

```rust
use flare_server_core::context::Context;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    let parent = Context::with_request_id("req-001");
    let child1 = parent.child();
    let child2 = parent.child();
    
    // 启动子任务
    let task1 = tokio::spawn(async move {
        loop {
            if child1.is_cancelled() {
                println!("Child 1 cancelled");
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    });
    
    let task2 = tokio::spawn(async move {
        loop {
            if child2.is_cancelled() {
                println!("Child 2 cancelled");
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    });
    
    // 等待一小段时间
    sleep(Duration::from_millis(50)).await;
    
    // 取消父 Context
    println!("Cancelling parent context...");
    parent.cancel();
    
    // 等待子任务完成
    let _ = tokio::try_join!(task1, task2)?;
    
    Ok(())
}
```

### 示例 3: 超时处理

```rust
use flare_server_core::context::Context;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = Context::with_request_id("req-004")
        .with_timeout(Duration::from_millis(100));

    // 模拟长时间运行的任务
    let result = tokio::select! {
        result = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok("task completed")
        } => result,
        _ = ctx.cancelled() => {
            Err(anyhow::anyhow!("task timeout"))
        }
    };

    match result {
        Ok(_) => println!("Task completed"),
        Err(e) => println!("Task failed: {}", e),
    }

    Ok(())
}
```

## 上下文类型支持

Context 系统支持以下上下文类型：

- **TenantContext**: 租户上下文（多租户隔离）
- **RequestContext**: 请求上下文（包含 trace、actor、device 等）
- **TraceContext**: 追踪上下文（分布式追踪）
- **ActorContext**: 操作者上下文（权限校验和审计）
- **DeviceContext**: 设备上下文（设备管理和统计，包含优先级、连接质量等）
- **AuditContext**: 审计上下文（合规和安全审计）

### 使用上下文类型

```rust
use flare_server_core::context::{Context, TenantContext, RequestContext, DeviceContext};

let ctx = Context::with_request_id("req-123")
    .with_tenant(TenantContext::new("tenant-456"))
    .with_request(RequestContext::new("req-123"))
    .with_device(DeviceContext::new("device-789"));

// 获取上下文
let tenant = ctx.tenant();
let request = ctx.request();
let device = ctx.device();
```

## 最佳实践

1. **总是传递 Context**：在异步函数中，Context 应该作为第一个参数传递
2. **使用子 Context**：为每个下游调用创建子 Context，确保取消传播
3. **定期检查取消**：在长时间运行的任务中，定期调用 `ctx.check_cancelled()`
4. **使用 select!**：在并发操作中，使用 `tokio::select!` 监听取消信号
5. **设置合理的超时**：为每个操作设置合理的超时时间
6. **存储常用资源**：将数据库连接、服务实例等常用资源存储在 Context 中，避免重复查找

## 注意事项

1. **Arc 共享**：Context 使用 `Arc` 内部共享，Clone 成本低，但修改需要创建新的 Context
2. **取消传播**：父 Context 取消时，所有子 Context 会自动取消
3. **超时继承**：子 Context 的截止时间不会晚于父 Context
4. **线程安全**：Context 是 `Send + Sync`，可以在多线程环境中使用
5. **自定义数据**：只能存储 `Send + Sync` 类型的数据
