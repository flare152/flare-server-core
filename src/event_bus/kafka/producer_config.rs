//! Kafka 生产者配置 Trait

/// Kafka 生产者配置
///
/// 任何需要构建 Kafka 生产者的服务配置或事件总线后端均可实现此 trait。
pub trait KafkaProducerConfig: Send + Sync {
    fn kafka_bootstrap(&self) -> &str;
    fn message_timeout_ms(&self) -> u64 {
        5000
    }
    fn enable_idempotence(&self) -> bool {
        true
    }
    fn compression_type(&self) -> &str {
        "snappy"
    }
    fn batch_size(&self) -> usize {
        64 * 1024
    }
    fn linger_ms(&self) -> u64 {
        10
    }
    fn retries(&self) -> u32 {
        3
    }
    fn retry_backoff_ms(&self) -> u64 {
        100
    }
    fn metadata_max_age_ms(&self) -> u64 {
        300000
    }
}
