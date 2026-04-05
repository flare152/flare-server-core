# MQ 框架完整指南

## 概述

`flare-server-core/mq` 模块提供了一个工业级的统一 MQ 框架，支持生产者和消费者，支持 Kafka 和 NATS JetStream，具有以下特性：

- **统一接口**: Producer 和 Consumer 使用统一的接口设计
- **多 MQ 支持**: Kafka 和 NATS JetStream
- **Context 透传**: 自动处理上下文信息（trace_id、user_id 等）
- **批量发送**: 支持批量发送消息，提高吞吐量
- **并发控制**: 消费者支持可配置的并发数
- **Topic 路由**: 自动按 topic 分发消息到对应 handler
- **优雅关闭**: 支持平滑的关闭流程

## 架构设计

### 分层架构

```
┌─────────────────────────────────────────────────────────┐
│                    业务应用层                            │
│           (domain/application/infrastructure)            │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│                  MQ 抽象层 (core)                        │
│  - producer.rs: 生产者抽象接口                          │
│  - consumer/: 消费者抽象接口和处理层                    │
└────────────────────┬────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────┐
│                 MQ 实现层 (impl)                         │
│  - kafka/: Kafka 具体实现（包含 Context 透传）          │
│  - nats/: NATS JetStream 具体实现（包含 Context 透传）  │
└─────────────────────────────────────────────────────────┘
```

### 模块结构

#### 抽象层 (Abstract Layer)

```
mq/
├── mod.rs                    # MQ 模块主文件
├── producer.rs               # 生产者抽象接口
└── consumer/                 # 消费者抽象接口和处理层
    ├── mod.rs                # consumer 模块主文件
    ├── types.rs              # 核心类型定义
    ├── handler.rs            # Handler 接口和注册表
    ├── dispatcher.rs         # 消息分发器
    └── runtime.rs            # 消费者运行时
```

#### 实现层 (Implementation Layer)

```
mq/
├── kafka/                    # Kafka 实现
│   ├── mod.rs
│   ├── config.rs
│   ├── producer.rs
│   └── consumer.rs
└── nats/                     # NATS JetStream 实现
    ├── mod.rs
    ├── config.rs
    ├── producer.rs
    └── consumer.rs
```

## 核心类型

### Producer 接口

```rust
#[async_trait]
pub trait Producer: Send + Sync {
    /// 发送消息
    async fn send(
        &self,
        ctx: &Ctx,
        topic: &str,
        key: Option<&str>,
        payload: Vec<u8>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), ProducerError>;

    /// 批量发送消息
    async fn send_batch(
        &self,
        ctx: &Ctx,
        messages: Vec<ProducerMessage>,
    ) -> Result<(), ProducerError>;

    fn name(&self) -> &str;
}
```

### Consumer Handler 接口

```rust
pub trait MessageHandler: Send + Sync {
    async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError>;
    fn name(&self) -> &str;
}
```

### Message 类型

```rust
pub struct Message {
    pub payload: Vec<u8>,
    pub content_type: ContentType,
    pub context: MessageContext,
}

pub struct MessageContext {
    pub ctx: Ctx,              // Context 对象（自动透传）
    pub message_id: String,
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub key: Option<String>,
    pub headers: HashMap<String, String>,
    pub started_at: Instant,
    pub retry_count: u32,
    pub metadata: HashMap<String, String>,
}
```

## Context 透传机制

### 自动 Context 透传

框架自动在 Producer 和 Consumer 之间透传 Context 信息：

**Producer 端自动添加到消息头**:
- `x-trace-id`: 追踪 ID
- `x-request-id`: 请求 ID
- `x-tenant-id`: 租户 ID
- `x-user-id`: 用户 ID
- `x-device-id`: 设备 ID
- `x-platform`: 平台信息
- `x-session-id`: 会话 ID
- `x-actor-id`: Actor ID
- `x-actor-type`: Actor 类型
- `x-actor-roles`: Actor 角色（逗号分隔）
- `x-actor-attr-*`: Actor 属性

**Consumer 端自动从消息头重建**:
- 自动解析所有 Context 字段
- 如果存在 trace_id 则复用，否则生成新 ID
- 构建完整的 Ctx 对象

## Kafka 使用

### Producer 使用

```rust
use flare_server_core::mq::producer::{Producer, ProducerConfig};
use flare_server_core::mq::kafka::{KafkaProducerBuilder, KafkaProducerConfig};
use flare_server_core::context::Context;

// 定义 Kafka 配置
struct MyKafkaConfig;

impl KafkaProducerConfig for MyKafkaConfig {
    fn kafka_bootstrap(&self) -> &str { "localhost:9092" }
    fn message_timeout_ms(&self) -> u64 { 5000 }
    fn enable_idempotence(&self) -> bool { true }
    fn compression_type(&self) -> &str { "snappy" }
    fn batch_size(&self) -> usize { 64 * 1024 }
    fn linger_ms(&self) -> u64 { 10 }
    fn retries(&self) -> u32 { 3 }
    fn retry_backoff_ms(&self) -> u64 { 100 }
    fn metadata_max_age_ms(&self) -> u64 { 300_000 }
}

// 创建生产者
let builder = KafkaProducerBuilder::new()
    .with_config(ProducerConfig::default());

let producer = builder.build(&MyKafkaConfig)?;

// 创建 Context
let ctx = Context::with_request_id("req-123".to_string())
    .with_trace_id("trace-456".to_string())
    .with_user_id("user-789".to_string());

// 发送消息（Context 会自动添加到消息头）
producer.send(
    &ctx,
    "test.topic",
    Some("key123"),
    b"Hello, Kafka!".to_vec(),
    None,
).await?;
```

### Consumer 使用

```rust
use flare_server_core::mq::consumer::{
    KafkaConsumerBuilder, ConsumerConfig, MessageHandler, Message,
    MessageResult, ConsumerError,
};
use async_trait::async_trait;

// 定义 Handler
struct MyMessageHandler;

#[async_trait::async_trait]
impl MessageHandler for MyMessageHandler {
    async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
        // Context 已自动从消息头中重建
        let ctx = &message.context;
        
        tracing::info!(
            "Processing message: trace_id = {}, user_id = {:?}",
            ctx.ctx.trace_id(),
            ctx.ctx.user_id()
        );

        // 处理业务逻辑
        Ok(MessageResult::Ack)
    }

    fn name(&self) -> &str {
        "my_message_handler"
    }
}

// 创建并启动消费者
let builder = KafkaConsumerBuilder::new()
    .with_config(
        ConsumerConfig::default()
            .with_concurrency(4)
            .with_idempotent(true)
    )
    .register_handler(
        MyMessageHandler,
        vec!["test.topic".to_string()],
    )?;

let runtime = builder.build();
runtime.start(kafka_consumer_config).await?;
```

## NATS JetStream 使用

### Producer 使用

```rust
use flare_server_core::mq::producer::{Producer, ProducerConfig};
use flare_server_core::mq::nats::{NatsProducerBuilder, NatsProducerConfig};
use flare_server_core::context::Context;

// 定义 NATS 配置
struct MyNatsConfig;

impl NatsProducerConfig for MyNatsConfig {
    fn nats_url(&self) -> &str { "nats://localhost:4222" }
    fn timeout_ms(&self) -> u64 { 5000 }
    fn retries(&self) -> u32 { 3 }
    fn retry_backoff_ms(&self) -> u64 { 100 }
}

// 创建生产者
let builder = NatsProducerBuilder::new()
    .with_config(ProducerConfig::default());

let producer = builder.build(&MyNatsConfig).await?;

// 创建 Context
let ctx = Context::with_request_id("req-123".to_string())
    .with_trace_id("trace-456".to_string());

// 发送消息（NATS 使用 subject 而不是 topic）
producer.send(
    &ctx,
    "events.user.created",
    None,
    b"{\"user_id\": \"123\"}".to_vec(),
    None,
).await?;
```

### Consumer 使用

```rust
use flare_server_core::mq::consumer::{
    NatsConsumerBuilder, ConsumerConfig, MessageHandler, Message,
    MessageResult, ConsumerError,
};
use async_trait::async_trait;

// 定义 Handler
struct UserCreatedHandler;

#[async_trait::async_trait]
impl MessageHandler for UserCreatedHandler {
    async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
        let ctx = &message.context;
        
        tracing::info!(
            "Processing user created event: trace_id = {}",
            ctx.ctx.trace_id()
        );

        // 处理业务逻辑
        Ok(MessageResult::Ack)
    }

    fn name(&self) -> &str {
        "user_created_handler"
    }
}

// 创建并启动消费者
let builder = NatsConsumerBuilder::new()
    .with_config(
        ConsumerConfig::default()
            .with_concurrency(4)
    )
    .register_handler(
        UserCreatedHandler,
        vec!["events.user.>".to_string()],  // NATS 支持通配符
    )?;

let runtime = builder.build();
runtime.start(nats_consumer_config).await?;
```

## 消息处理结果

```rust
pub enum MessageResult {
    /// 处理成功，确认消息
    Ack,
    /// 处理失败，拒绝消息（将返回队列）
    Nack,
    /// 处理失败，发送到 DLQ
    DeadLetter,
}
```

## 错误处理

### Producer 错误

```rust
pub enum ProducerError {
    Connection(String),      // 连接错误
    Serialization(String),   // 序列化错误
    Timeout(String),         // 超时错误
    Configuration(String),   // 配置错误
    Send(String),            // 发送错误
    Batch(String),           // 批处理错误
}
```

### Consumer 错误

```rust
pub enum ConsumerError {
    Handler(String),         // Handler 处理错误
    Serialization(String),   // 序列化错误
    Deserialization(String), // 反序列化错误
    Connection(String),      // 连接错误（可重试）
    Timeout(String),         // 超时错误（可重试）
    DeadLetter(String),      // DLQ 错误
    Shutdown,                // 关闭请求
    Configuration(String),   // 配置错误
    NoHandler(String),       // 未找到 Handler
}

impl ConsumerError {
    /// 判断错误是否可重试
    pub fn is_retryable(&self) -> bool;

    /// 判断是否应该发送到 DLQ
    pub fn should_dead_letter(&self) -> bool;
}
```

## 最佳实践

### 1. 幂等性处理

```rust
async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
    let message_id = &message.context.message_id;

    // 检查是否已处理
    if self.is_processed(message_id) {
        return Ok(MessageResult::Ack);
    }

    // 处理消息
    self.process(&message).await?;

    // 标记为已处理
    self.mark_processed(message_id);

    Ok(MessageResult::Ack)
}
```

### 2. Context 使用

```rust
async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
    let ctx = &message.context;

    // 使用 trace_id 进行日志追踪
    tracing::info!(
        trace_id = %ctx.ctx.trace_id(),
        user_id = %ctx.ctx.user_id().unwrap_or("anonymous"),
        "Processing message"
    );

    // 使用租户 ID 进行数据隔离
    let tenant_id = ctx.ctx.tenant_id().ok_or_else(|| {
        ConsumerError::Configuration("Missing tenant_id".to_string())
    })?;

    // 处理业务逻辑
    self.process_with_tenant(tenant_id, &message).await?;

    Ok(MessageResult::Ack)
}
```

### 3. 错误处理

```rust
async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
    match self.process(&message).await {
        Ok(_) => Ok(MessageResult::Ack),
        Err(e) if e.is_transient() => {
            // 可重试错误，返回 Nack
            Ok(MessageResult::Nack)
        }
        Err(e) => {
            // 不可恢复错误，发送到 DLQ
            tracing::error!("Permanent error: {}", e);
            Ok(MessageResult::DeadLetter)
        }
    }
}
```

## 配置选项

### Producer 配置

```rust
let config = ProducerConfig::default()
    .with_timeout_ms(5000)           // 超时时间
    .with_idempotence(true)          // 启用幂等性
    .with_compression("snappy")       // 压缩类型
    .with_batch_size(64 * 1024)      // 批处理大小
    .with_retries(3);                 // 重试次数
```

### Consumer 配置

```rust
let config = ConsumerConfig::default()
    .with_concurrency(8)           // 并发消费数
    .with_batch_size(10)           // 批处理大小
    .with_ordered(true)            // 启用顺序消费
    .with_idempotent(true);        // 启用幂等性检查
```

## Kafka vs NATS

| 特性 | Kafka | NATS JetStream |
|------|-------|----------------|
| 消息模型 | Topic/Partition | Subject/Stream |
| 消息键 | 支持 | 通过 Header |
| 分区 | 支持 | 不支持 |
| 持久化 | 支持 | 支持 |
| 消息顺序 | 分区内有序 | Stream 内有序 |
| 通配符 | 不支持 | 支持 |
| Context 透传 | 支持 | 支持 |

## 注意事项

1. **线程安全**: Producer 和 Handler 必须是线程安全的
2. **异步优先**: 避免阻塞操作，使用异步 API
3. **错误处理**: 正确处理错误，区分可重试和不可恢复错误
4. **Context 透传**: 始终通过 Context 传递追踪信息
5. **幂等性**: 建议实现幂等性检查以避免重复处理

## 扩展新 MQ 实现

要添加新的 MQ 实现，只需：

1. 创建 `mq/newmq/` 目录
2. 实现配置 Trait
3. 实现 `Producer` trait
4. 实现 `MessageFetcher` trait
5. 实现 Context 透传逻辑

## 总结

这个 MQ 框架提供了：

- **清晰的分层架构**: 抽象层和实现层完全分离
- **统一的接口**: 所有 MQ 实现共享相同的接口
- **Context 透传**: 自动处理上下文信息的编码和解码
- **多 MQ 支持**: Kafka 和 NATS JetStream
- **易于扩展**: 添加新 MQ 实现不需要修改抽象层
- **生产级**: 支持幂等性、重试、优雅关闭等生产环境必需功能
