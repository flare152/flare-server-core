//! Kafka 消费者配置 Trait

/// Kafka 消费者配置
///
/// 任何需要构建 Kafka 消费者的服务配置均可实现此 trait。
pub trait KafkaConsumerConfig: Send + Sync {
    fn kafka_bootstrap(&self) -> &str;
    fn consumer_group(&self) -> &str;
    fn kafka_topic(&self) -> &str;
    fn fetch_min_bytes(&self) -> usize;
    fn fetch_max_wait_ms(&self) -> u64;
    fn session_timeout_ms(&self) -> u64 {
        30000
    }
    fn enable_auto_commit(&self) -> bool {
        false
    }
    fn auto_offset_reset(&self) -> &str {
        "earliest"
    }
    fn fetch_message_max_bytes(&self) -> usize {
        10 * 1024 * 1024
    }
    fn max_partition_fetch_bytes(&self) -> usize {
        10 * 1024 * 1024
    }
    fn metadata_max_age_ms(&self) -> u64 {
        300000
    }
}
