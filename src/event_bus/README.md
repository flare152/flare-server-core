# 事件总线模块（Event Bus）

## 1. 放置位置：为什么在 flare-server-core 而不是 flare-im-core

**结论：事件总线抽象与 Kafka 实现放在 flare-server-core 是合适的。**

| 维度 | server-core | im-core |
|------|-------------|---------|
| **定位** | 服务端开发基础（多产品复用） | IM 业务领域与编排 |
| **事件总线** | 通用基础设施：trait、信封、分区/消费组语义 | 业务侧：topic 名、event_type 常量、谁发布/谁消费 |
| **类比** | 与 `discovery`、`kv`、`kafka`、`context`、`runtime` 同级 | 与「消息协议」「会话模型」同级 |

**理由简述：**

- **TopicEventBus**、**EventEnvelope**、分区键/consumer_group 等与具体业务无关，任何微服务（IM、推送、审计、工单）都可复用。
- 若把 trait/实现放在 im-core，其他非 IM 产品要复用同一套事件总线就需要依赖 im-core，违反「基础库与业务解耦」。
- im-core 应只保留：`TOPIC_*`、`EVENT_TYPE_*` 常量，以及各服务对 payload 的解析与处理（例如 StoreMessage / Event）。

因此：**标准 trait + 信封 + Kafka 实现放在 server-core；im-core 只定义「用哪些 topic、发哪些 event_type」并依赖 server-core 的 event_bus。**

---

## 2. 模块拆分：标准 trait 与 Kafka 实现是否分多模块

**结论：抽象与实现已通过 feature 分离；建议在「导出」和「目录约定」上再收紧一步。**

### 2.1 当前结构（合理部分）

- **抽象**：`envelope.rs`（EventEnvelope）、`topic_bus.rs`（TopicEventBus、TopicEventBusError）—— 无 rdkafka 依赖。
- **实现**：`kafka_topic.rs` 在 `#[cfg(feature = "kafka")]` 下，依赖 rdkafka。
- 进程内总线：`event_bus.rs`（DomainEvent / InMemoryEventBus）与分布式总线并列，互不干扰。

### 2.2 模块边界（已落地）

| 层次 | 内容 | 依赖 | 导出 |
|------|------|------|------|
| **核心抽象** | `envelope`、`topic_bus`（trait + error） | 仅 std/serde/async-trait/futures | **默认导出** |
| **Kafka 实现** | `backend/kafka`（KafkaTopicEventBus、KafkaEventBusConfig） | rdkafka | **仅 `kafka` feature** |
| **进程内** | `event_bus`（InMemoryEventBus 等） | 无 kafka | 默认导出 |

不启用 `kafka` 的 crate 可使用 `TopicEventBus`、`EventEnvelope` 并自实现内存/测试或其它后端（如 NATS）。

### 2.3 目录结构（已落地）

- **核心抽象**：`envelope.rs`、`topic_bus.rs`（无 Kafka 依赖，默认导出）
- **Kafka 传输**（feature `kafka`）：`event_bus/kafka/` — 通用生产者/消费者构建（`KafkaProducerConfig`、`KafkaConsumerConfig`、`build_kafka_producer`、`build_kafka_consumer`），供各服务及事件总线后端复用
- **事件总线 Kafka 后端**：`event_bus/backend/kafka.rs` — `KafkaTopicEventBus`、`KafkaEventBusConfig`（丰富配置，实现 `KafkaProducerConfig` 并复用 `build_kafka_producer`）
- **lib.rs**：默认导出事件总线抽象；`kafka` feature 时 `pub use event_bus::kafka`，保持 `flare_server_core::kafka::*` 兼容

---

## 3. EventEnvelope 的通用性（与业务解耦）

**EventEnvelope** 不绑定 IM，适用于任意基于「分区流」的事件场景：

| 字段 / 方法 | 含义 | IM 用法示例 | 其他业务示例 |
|-------------|------|-------------|--------------|
| `partition_key` | 分区键，同 key 内有序 | `conversation_id` | `order_id`、`tenant_id`、`user_id` |
| `event_type` | 业务自定义类型 | `message.created`、`operation.recalled` | `order.shipped`、`audit.logged` |
| `seq` | 流内序号 | 会话内消息序 | 订单内操作序 |
| `payload` | 业务载荷 | StoreMessage / Event 等 proto | JSON / 自定义编码 |
| `is_operation()` | 可选约定（前缀 `operation.`） | 操作类事件 | 可忽略或自定前缀 |

业务侧只需在构造时传入自己的 `partition_key`（如 IM 传 `conversation_id`），无需依赖 IM 概念。

---

## 4. 小结

- **放哪里**：事件总线（trait + envelope + Kafka 实现）放在 **flare-server-core**；**flare-im-core** 只做 topic/event_type 常量与业务侧使用。
- **怎么拆**：标准 trait 与 Kafka 实现已分文件且 feature 隔离；EventEnvelope、TopicEventBus、TopicEventBusError 默认导出，Kafka 实现仅随 `kafka` feature 导出。
- **信封通用性**：使用 `partition_key` 而非 `conversation_id`，event_type / payload 由业务定义，server-core 不依赖 IM。
