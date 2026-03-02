# Context 系统实现总结

## 实现概述

在 `flare-server-core` 中实现了一个类型安全、显式传递的上下文系统，支持取消、超时和自定义数据存储。

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

## 文件结构

```
flare-server-core/src/context/
├── mod.rs              # 模块入口，定义所有上下文类型
├── core.rs             # 核心 Context 实现
├── typemap.rs          # 类型映射表（自定义数据存储）
├── conversions.rs      # 与 proto 的转换（可选）
├── examples.rs         # 使用示例
└── README.md           # 详细使用文档
```

## 核心 API

### Context 创建

```rust
// 根 Context
let ctx = Context::root();

// 带请求 ID
let ctx = Context::with_request_id("req-123");

// 链式调用
let ctx = Context::with_request_id("req-123")
    .with_trace_id("trace-456")
    .with_user_id("user-789")
    .with_tenant_id("tenant-abc");
```

### 自定义数据存储

```rust
// 存储数据库连接
let db_pool: Arc<PgPool> = get_db_pool();
let ctx = ctx.insert_data(db_pool);

// 获取自定义数据
let pool: Option<&Arc<PgPool>> = ctx.get_data();
```

### 上下文类型支持

```rust
// 设置上下文
let ctx = ctx
    .with_tenant(TenantContext::new("tenant-123"))
    .with_request(RequestContext::new("req-123"))
    .with_trace(TraceContext::new("trace-456"))
    .with_actor(ActorContext::new("user-789"))
    .with_device(DeviceContext::new("device-abc"));

// 获取上下文
let tenant = ctx.tenant();
let request = ctx.request();
let trace = ctx.trace();
let actor = ctx.actor();
let device = ctx.device();
```

### 子 Context

```rust
// 创建子 Context
let child = parent.child();

// 带超时
let timeout_ctx = parent.with_timeout(Duration::from_secs(5));

// 带截止时间
let deadline_ctx = parent.with_deadline(Instant::now() + Duration::from_secs(10));
```

### 取消和检查

```rust
// 取消
ctx.cancel();

// 检查
if ctx.is_cancelled() { ... }

// 等待取消
ctx.cancelled().await;
```

### 在 select! 中使用

```rust
tokio::select! {
    result = do_work() => result,
    _ = ctx.cancelled() => Err(anyhow::anyhow!("cancelled")),
}
```

## 集成点

### 1. gRPC 客户端 (`src/client/context.rs`)

```rust
use flare_server_core::client::{set_context_metadata, request_with_context};

// 设置 Context 到请求（自动构建 metadata）
let mut request = Request::new(payload);
set_context_metadata(&mut request, &ctx);

// 或使用便捷函数
let request = request_with_context(payload, &ctx);
```

### 2. gRPC 服务端中间件 (`src/middleware/context.rs`)

```rust
use flare_server_core::middleware::{ContextLayer, extract_context};

// 在 bootstrap 中应用（兼容现有中间件）
Server::builder()
    .layer(TenantLayer::new().allow_missing())
    .layer(RequestContextLayer::new().allow_missing())
    .layer(ContextLayer::new().allow_missing()) // 放在最后
    .add_service(MyServiceServer::new(handler))
    .serve(addr)
    .await?;

// 在 handler 中提取
let ctx = extract_context(&request)?;
```

### 3. 上下文类型转换

```rust
// 从 RequestContext 创建 Context
let ctx = Context::from(&req_ctx);

// 从 TenantContext 创建 Context
let ctx = Context::from(&tenant);

// 从 Context 创建 RequestContext（自动构建）
let req_ctx: RequestContext = (&ctx).into();

// 从 Context 创建 TenantContext（自动构建）
let tenant: TenantContext = (&ctx).into();
```

## 设计决策

### 为什么使用 `watch::channel` 而不是 `CancellationToken`？

1. **兼容性**：`watch::channel` 是 tokio 核心功能，无需额外依赖
2. **简单性**：实现简单，易于理解和维护
3. **灵活性**：可以轻松扩展支持更多取消原因

### 为什么使用显式字段而不是 `HashMap<String, Any>`？

1. **类型安全**：编译时检查，避免运行时错误
2. **性能**：直接字段访问，无需哈希查找
3. **可维护性**：字段清晰，IDE 自动补全

### 为什么使用 `Arc<Inner>` 而不是直接字段？

1. **Clone 成本**：`Arc` 的 Clone 是 O(1)，成本低
2. **共享**：多个 Context 可以共享同一个 `Inner`
3. **不可变**：`Inner` 不可变，修改需要创建新的 Context

### 为什么使用 `TypeMap` 存储自定义数据？

1. **类型安全**：每个类型只能存储一个值，编译时检查
2. **零成本抽象**：使用 `TypeId` 作为键，无需字符串比较
3. **灵活性**：支持存储任意 `Send + Sync` 类型

## 测试覆盖

所有核心功能都有测试覆盖：

- ✅ `test_root_context` - 根 Context 创建
- ✅ `test_cancel` - 取消功能
- ✅ `test_child_context` - 父子关系
- ✅ `test_timeout` - 超时功能
- ✅ `test_cancelled_future` - 取消 Future
- ✅ `test_remaining_time` - 剩余时间
- ✅ `test_with_metadata` - 元数据设置
- ✅ `test_select_with_context` - select! 集成

## 使用示例

详细的使用示例请参考：
- `src/context/examples.rs` - 代码示例
- `src/context/README.md` - 详细文档

## 下一步

1. ✅ 核心 Context 实现
2. ✅ 自定义数据存储（TypeMap）
3. ✅ 上下文类型支持（TenantContext、RequestContext 等）
4. ✅ 客户端集成
5. ✅ 服务端中间件集成
6. ✅ 测试覆盖
7. ✅ 文档完善

## 注意事项

1. **Arc 共享**：Context 使用 `Arc` 内部共享，Clone 成本低
2. **取消传播**：父 Context 取消时，所有子 Context 会自动取消
3. **超时继承**：子 Context 的截止时间不会晚于父 Context
4. **线程安全**：Context 是 `Send + Sync`，可以在多线程环境中使用
5. **自定义数据**：只能存储 `Send + Sync` 类型的数据
