# Kafka 工具模块

## 概述

提供通用的 Kafka 消费者和生产者构建工具，支持所有服务复用。采用 **Trait 抽象** 设计，避免依赖具体的配置结构体。

## 设计原则

### 1. **依赖倒置原则 (DIP)**
- 使用 `KafkaConsumerConfig` trait 抽象配置依赖
- 不依赖具体的配置结构体（如 `PushServerConfig`, `StorageWriterConfig`）
- 任何实现了 trait 的配置都可以使用构建器

### 2. **单一职责原则 (SRP)**
- `consumer_config.rs`: 定义消费者配置接口
- `consumer_builder.rs`: 提供消费者构建和订阅逻辑
- `producer_config.rs`: 定义生产者配置接口
- `producer_builder.rs`: 提供生产者构建逻辑

### 3. **开闭原则 (OCP)**
- 对扩展开放：新服务只需实现 `KafkaConsumerConfig` 或 `KafkaProducerConfig` trait
- 对修改封闭：核心构建逻辑不需要修改

## 使用方式

### 1. 实现 `KafkaConsumerConfig` trait

```rust
use flare_server_core::kafka::KafkaConsumerConfig;

impl KafkaConsumerConfig for YourConfig {
    fn kafka_bootstrap(&self) -> &str {
        &self.kafka_bootstrap
    }
    
    fn consumer_group(&self) -> &str {
        &self.consumer_group
    }
    
    fn kafka_topic(&self) -> &str {
        &self.kafka_topic
    }
    
    fn fetch_min_bytes(&self) -> usize {
        self.fetch_min_bytes
    }
    
    fn fetch_max_wait_ms(&self) -> u64 {
        self.fetch_max_wait_ms
    }
    
    // 可选：覆盖默认值
    fn session_timeout_ms(&self) -> u64 {
        6000 // 自定义超时
    }
}
```

### 2. 使用消费者构建器

```rust
use flare_server_core::kafka::{build_kafka_consumer, subscribe_and_wait_for_assignment};

// 构建消费者
let consumer = build_kafka_consumer(&config)?;

// 订阅并等待 partition assignment
subscribe_and_wait_for_assignment(&consumer, &config.kafka_topic(), 15).await?;
```

### 3. 实现 `KafkaProducerConfig` trait

```rust
use flare_server_core::kafka::KafkaProducerConfig;

impl KafkaProducerConfig for YourConfig {
    fn kafka_bootstrap(&self) -> &str {
        &self.kafka_bootstrap
    }
    
    fn message_timeout_ms(&self) -> u64 {
        self.kafka_timeout_ms
    }
    
    // 可选：覆盖默认值
    fn enable_idempotence(&self) -> bool {
        true // 启用幂等性，保证消息不丢失
    }
    
    fn compression_type(&self) -> &str {
        "snappy" // 使用 snappy 压缩
    }
}
```

### 4. 使用生产者构建器

```rust
use flare_server_core::kafka::build_kafka_producer;

// 构建生产者
let producer = build_kafka_producer(&config)?;
```

## 配置项说明

### 必需配置（必须实现）
- `kafka_bootstrap()`: Kafka 服务器地址
- `consumer_group()`: Consumer Group ID
- `kafka_topic()`: Topic 名称
- `fetch_min_bytes()`: 最小 fetch 字节数
- `fetch_max_wait_ms()`: 最大 fetch 等待时间

### 可选配置（有默认值）
- `session_timeout_ms()`: 会话超时，默认 30000ms
- `enable_auto_commit()`: 是否自动提交，默认 false
- `auto_offset_reset()`: Offset 重置策略，默认 "earliest"
- `fetch_message_max_bytes()`: 最大消息大小，默认 10MB
- `max_partition_fetch_bytes()`: 最大分区 fetch 大小，默认 10MB
- `metadata_max_age_ms()`: 元数据最大年龄，默认 5 分钟

## 生产者配置项说明

### 必需配置（必须实现）
- `kafka_bootstrap()`: Kafka 服务器地址
- `message_timeout_ms()`: 消息超时时间（毫秒）

### 可选配置（有默认值）
- `enable_idempotence()`: 是否启用幂等性，默认 true（推荐）
- `compression_type()`: 压缩类型，默认 "snappy"
- `batch_size()`: 批量发送大小，默认 64KB
- `linger_ms()`: 批量发送延迟，默认 10ms
- `retries()`: 重试次数，默认 3
- `retry_backoff_ms()`: 重试间隔，默认 100ms
- `metadata_max_age_ms()`: 元数据最大年龄，默认 5 分钟

**注意**: 当 `enable_idempotence()` 返回 `true` 时，构建器会自动设置 `acks=all`，确保消息不丢失。

## 特性

1. **自动 Fallback**: 如果订阅失败，自动尝试手动分配 partition 0
2. **Partition Assignment 等待**: 自动等待 partition assignment，最多等待 15 秒
3. **统一配置**: 所有服务使用相同的配置项和默认值
4. **可扩展**: 新服务只需实现 trait，无需修改核心代码

## 优势

### 代码复用
- ✅ 消除重复代码
- ✅ 统一配置管理
- ✅ 统一错误处理

### 可维护性
- ✅ 配置集中管理
- ✅ 易于测试（构建器可独立测试）
- ✅ 易于扩展（新服务只需实现 trait）

### 性能
- ✅ 优化的默认配置
- ✅ 合理的超时设置
- ✅ 高效的 partition assignment 逻辑

## 已使用的服务

### 消费者
- ✅ `flare-push-server`
- ✅ `flare-storage-writer`

### 生产者
- ✅ `flare-push-server` (KafkaPushTaskPublisher)
- ✅ `flare-storage-writer` (ACK Publisher)

## 未来扩展

可以考虑添加：
- 批量消费工具
- 消息序列化/反序列化工具
- 指标收集集成
- 生产者性能监控

