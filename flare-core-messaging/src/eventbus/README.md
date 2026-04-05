# event_bus 使用说明

本模块提供：

1. **进程内领域事件**：`DomainEvent` + `EventBus<E>` + `InMemoryEventBus`（按事件类型 `E` 广播）。
2. **Topic 事件（跨模块 / 跨服务）**：`TopicEventBus` + `EventEnvelope`，两种实现：
   - **`InMemoryTopicEventBus`**：同进程 `broadcast` 扇出（测试、单机编排）。
   - **`MqTopicEventBus`**：走 `mq::Producer` + 可选 `mq::consumer` 消费链。

**错误约定**：`TopicEventBus::publish`、`TopicEnvelopeHandler::handle`、进程内 `EventBus::publish` / `DomainEvent::to_json` 均使用 **`crate::error::Result`**（`FlareError`）。  
`mq::consumer` 层仍使用 **`ConsumerError`**；在 `TopicEnvelopeMessageHandler` 内会将 `FlareError` 转为 `ConsumerError::Handler` 字符串。

**其它约定**：`MqTopicEventBus::publish` 使用信封的 `to_json_bytes()` 与 `constants` 中的头字段；`register_topic_envelope_dispatcher` 的 `topics` 参数为 **`IntoIterator<Item = String>`**（`vec![..]`、`["a".into(), ...]` 均可）。内存总线可用 **`InMemoryTopicEventBus::new_default()`**（容量见 **`DEFAULT_TOPIC_BROADCAST_CAPACITY`**）或 `receiver_count()` 做观测。

---

## 1. 模块与文件

| 文件 | 说明 |
|------|------|
| `constants.rs` | `EVENT_ENVELOPE_CONTENT_TYPE`、`HEADER_CONTENT_TYPE`（与 `mq::consumer::ContentType::Json` 一致） |
| `domain_event_bus.rs` | `DomainEvent`、`EventSubscriber`、`EventBus` |
| `in_memory_domain_event_bus.rs` | `InMemoryEventBus` |
| `envelope.rs` | `EventEnvelope`（含 `to_json_bytes()` 统一 JSON 编码） |
| `topic_event_bus.rs` | `TopicEventBus` trait（含对 `Arc<P>` 的委托实现） |
| `in_memory_topic_event_bus.rs` | `InMemoryTopicEventBus`、`TopicBroadcast`、`new_default` / `receiver_count` |
| `mq_topic.rs` | `MqTopicEventBus`、`TopicEnvelopeHandler`、dispatcher 注册与 `run_topic_event_consumer` |

Kafka 具体类型见 **`mq::kafka`** 或 **`flare_server_core::kafka`**（需 `kafka` feature）。

---

## 2. 示例 A：内存 `TopicEventBus` —— 一条消息，两个订阅（持久化 + 更新会话）

典型模式：**发布一条** `EventEnvelope`，**两个** `subscribe()` 循环各自过滤同一 `topic`，分别写库与更新读模型（会话摘要等）。

```rust,ignore
use std::sync::Arc;
use flare_server_core::context::Context;
use flare_server_core::error::Result;
use flare_server_core::{
    EventEnvelope, InMemoryTopicEventBus, TopicBroadcast, TopicEventBus,
};
use tokio::sync::broadcast::error::RecvError;

const TOPIC: &str = "im.message.events";

async fn run_in_memory_fanout_example() -> Result<()> {
    let bus = InMemoryTopicEventBus::new(256);
    let mut rx_save = bus.subscribe();
    let mut rx_session = bus.subscribe();

    // 订阅 1：落库（示意）
    tokio::spawn(async move {
        loop {
            match rx_save.recv().await {
                Ok(msg) if msg.topic.as_ref() == TOPIC => {
                    let _ = persist_message_projection(&msg).await;
                }
                Ok(_) => {}
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => { /* 可打 metrics */ }
            }
        }
    });

    // 订阅 2：更新会话读模型（示意）
    tokio::spawn(async move {
        loop {
            match rx_session.recv().await {
                Ok(msg) if msg.topic.as_ref() == TOPIC => {
                    let _ = refresh_conversation_summary(&msg).await;
                }
                Ok(_) => {}
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => {}
            }
        }
    });

    let ctx = Arc::new(Context::with_request_id("req-1").with_trace_id("t1"));
    let envelope = EventEnvelope::new("message.created", "conv-abc", 42, vec![]);
    bus.publish(&ctx, TOPIC, &envelope).await?;
    Ok(())
}

async fn persist_message_projection(_msg: &TopicBroadcast) -> Result<()> {
    // sqlx / repository …
    Ok(())
}

async fn refresh_conversation_summary(_msg: &TopicBroadcast) -> Result<()> {
    // 更新 Redis / 本地缓存中的会话摘要 …
    Ok(())
}
```

要点：

- **`publish(ctx, topic, envelope)`** 与 MQ 版签名一致，`ctx` 会进入 **`TopicBroadcast`**。
- 多订阅者需处理 **`RecvError::Lagged`**（消费过慢会丢历史，与 `broadcast` 语义一致）。

---

## 3. 示例 B：MQ `TopicEventBus` —— 发布 + 消费侧「保存 + 更新会话」

`TopicDispatcher` **每个 topic 仅注册一个** `MessageHandler`。若同一 topic 上既要落库又要更新会话，推荐：

- **一个** `TopicEnvelopeHandler` 内**顺序**调用两段逻辑（或内部再 `spawn` 解耦），或  
- 拆成**多个 topic**（如 `…events` / `…session`）由上游分别发送。

下面演示 **组合 Handler**：一次消费内先 `persist` 再 `touch_session`。

```rust,ignore
use std::sync::Arc;
use async_trait::async_trait;
use flare_server_core::context::Ctx;
use flare_server_core::error::Result;
use flare_server_core::event_bus::{
    EventEnvelope, MqTopicEventBus, TopicEnvelopeHandler, TopicEventBus,
    register_topic_envelope_dispatcher, run_topic_event_consumer,
};
use flare_server_core::mq::consumer::ConsumerConfig;
// kafka feature 下：
// use flare_server_core::kafka::{KafkaMessageFetcher, KafkaConsumerConfig, KafkaProducer, ...};

struct SaveAndSessionPipeline {
    /* repo: Arc<MessageRepo>, session: Arc<SessionReadModel>, */
}

#[async_trait]
impl TopicEnvelopeHandler for SaveAndSessionPipeline {
    async fn handle(&self, ctx: &Ctx, envelope: EventEnvelope) -> Result<()> {
        // 1) 持久化 / 投影
        persist_from_envelope(ctx, &envelope).await?;
        // 2) 更新会话读模型
        update_session_from_envelope(ctx, &envelope).await?;
        Ok(())
    }

    fn name(&self) -> &str {
        "save_and_session_pipeline"
    }
}

async fn persist_from_envelope(_ctx: &Ctx, _env: &EventEnvelope) -> Result<()> {
    Ok(())
}

async fn update_session_from_envelope(_ctx: &Ctx, _env: &EventEnvelope) -> Result<()> {
    Ok(())
}

async fn run_mq_pipeline_example() -> Result<()> {
    // let producer = Arc::new(KafkaProducer::new(&producer_cfg)?);
    // let bus = MqTopicEventBus::new(producer);
    // let ctx = Arc::new(Context::with_request_id("r1"));
    // bus.publish(&ctx, "im.message.events", &envelope).await?;

    let handler = Arc::new(SaveAndSessionPipeline { /* … */ });
    let dispatcher = register_topic_envelope_dispatcher(
        handler,
        vec!["im.message.events".into()],
    )
    .map_err(|e| flare_server_core::error::FlareError::system(e.to_string()))?;

    // let fetcher = KafkaMessageFetcher::new(&consumer_cfg, vec!["im.message.events".into()])?;
    // run_topic_event_consumer(
    //     fetcher,
    //     Arc::new(dispatcher),
    //     ConsumerConfig::default(),
    // )
    // .await
    // .map_err(|e| flare_server_core::error::FlareError::system(e.to_string()))?;

    Ok(())
}
```

要点：

- 发布与内存版相同：**`bus.publish(&ctx, topic, &envelope)`**；`KafkaProducer` 会把 `ctx` 写入头。
- 消费：`TopicEnvelopeHandler::handle` 返回 **`Result<(), FlareError>`**；若需 `?` 转换 `ConsumerError`，在外层 `run_topic_event_consumer` 的 `map_err` 中处理（见注释）。

---

## 4. `EventEnvelope` 字段（摘要）

| 字段 | 含义 |
|------|------|
| `event_id` | 唯一 ID（`new` 时自动生成） |
| `event_type` | 业务类型，如 `message.created` |
| `partition_key` | Kafka 等用作 message key，保证同键有序 |
| `seq` | 流内序号 |
| `payload` | 业务字节（JSON 或 proto `encode_to_vec`） |

---

## 5. 进程内领域事件（`DomainEvent` / `InMemoryEventBus`）

与 Topic 总线**独立**：面向**强类型**领域事件 `E`，不经过 `EventEnvelope`。

1. 为 `E` 实现 `DomainEvent`（`to_json` 返回 **`crate::error::Result<String>`**）。  
2. `InMemoryEventBus::<E>::new(capacity)`。  
3. `publish` / `subscribe` 使用 `broadcast::Receiver<E>`。

---

## 6. 错误与 `FlareError` 选用

| 场景 | 建议 |
|------|------|
| JSON 序列化失败（发布前） | `FlareError::serialization_error` |
| MQ 发送失败 | `FlareError::message_send_failed` |
| 业务拒绝 / 领域规则 | `FlareError::localized` + 合适 `ErrorCode`，或 `FlareError::system`（内部） |
| 内存 broadcast 无接收端 | `FlareError::message_send_failed`（`InMemoryEventBus` / `InMemoryTopicEventBus` 已映射） |

---

## 7. 设计小结

- **Topic 内存 vs MQ**：同一 **`TopicEventBus`** trait，便于测试换实现。  
- **Ctx**：发布侧显式传入；内存订阅在 **`TopicBroadcast.ctx`**；MQ 订阅在 **`TopicEnvelopeHandler::handle` 的第一个参数**。  
- **两订阅者**：内存用**两个 `subscribe()`**；MQ 用**组合 Handler** 或多 topic，避免违反「每 topic 单 Handler」的 dispatcher 约束。
