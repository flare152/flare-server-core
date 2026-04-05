//! Topic 事件总线与 MQ 生产侧对齐的约定常量（避免与 `mq::consumer::ContentType::Json` 漂移）

/// 与 [crate::mq::consumer::ContentType::Json] 的 MIME 一致，写入 Kafka/NATS 等消息头
pub const EVENT_ENVELOPE_CONTENT_TYPE: &str = "application/json";

/// 与 [crate::mq::consumer::ContentType::Protobuf] 的 MIME 一致，写入 Kafka/NATS 等消息头
pub const EVENT_ENVELOPE_PROTO_CONTENT_TYPE: &str = "application/protobuf";

/// 标准消息头键（小写，与 `KafkaProducer` 等一致）
pub const HEADER_CONTENT_TYPE: &str = "content-type";
