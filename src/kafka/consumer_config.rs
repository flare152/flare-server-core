//! Kafka 消费者配置 Trait
//!
//! 定义通用的 Kafka 消费者配置接口，允许不同服务提供自己的配置实现

/// Kafka 消费者配置 Trait
///
/// 任何需要构建 Kafka 消费者的服务配置都应该实现此 trait
pub trait KafkaConsumerConfig: Send + Sync {
    /// Kafka Bootstrap Servers 地址
    fn kafka_bootstrap(&self) -> &str;
    
    /// Consumer Group ID
    fn consumer_group(&self) -> &str;
    
    /// Kafka Topic 名称
    fn kafka_topic(&self) -> &str;
    
    /// 最小 fetch 字节数
    fn fetch_min_bytes(&self) -> usize;
    
    /// 最大 fetch 等待时间（毫秒）
    fn fetch_max_wait_ms(&self) -> u64;
    
    /// 会话超时（毫秒），可选，默认 30000
    fn session_timeout_ms(&self) -> u64 {
        30000
    }
    
    /// 是否自动提交 offset，默认 false
    fn enable_auto_commit(&self) -> bool {
        false
    }
    
    /// Offset 重置策略，默认 "earliest"
    fn auto_offset_reset(&self) -> &str {
        "earliest"
    }
    
    /// 最大消息大小（字节），默认 10MB
    fn fetch_message_max_bytes(&self) -> usize {
        10 * 1024 * 1024
    }
    
    /// 最大分区 fetch 大小（字节），默认 10MB
    fn max_partition_fetch_bytes(&self) -> usize {
        10 * 1024 * 1024
    }
    
    /// 元数据最大年龄（毫秒），默认 5 分钟
    fn metadata_max_age_ms(&self) -> u64 {
        300000
    }
}

