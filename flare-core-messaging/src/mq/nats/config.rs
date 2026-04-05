//! NATS JetStream 配置 Trait

/// NATS JetStream 生产者配置 Trait
pub trait NatsProducerConfig: Send + Sync {
    /// NATS 服务器 URL
    fn nats_url(&self) -> &str;

    /// 超时时间（毫秒）
    fn timeout_ms(&self) -> u64;

    /// 重试次数
    fn retries(&self) -> u32;

    /// 重试退避时间（毫秒）
    fn retry_backoff_ms(&self) -> u64;
}

/// NATS JetStream 消费者配置 Trait
pub trait NatsConsumerConfig: Send + Sync {
    /// NATS 服务器 URL
    fn nats_url(&self) -> &str;

    /// Consumer group ID
    fn consumer_group(&self) -> &str;

    /// 是否启用手动确认
    fn enable_manual_ack(&self) -> bool;

    /// 批处理大小
    fn batch_size(&self) -> usize;

    /// 批处理超时（毫秒）
    fn batch_timeout_ms(&self) -> u64;

    /// 是否启用持久化
    fn enable_durable(&self) -> bool;
}
