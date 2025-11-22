//! Kafka 生产者配置 Trait
//!
//! 定义通用的 Kafka 生产者配置接口，允许不同服务提供自己的配置实现

/// Kafka 生产者配置 Trait
///
/// 任何需要构建 Kafka 生产者的服务配置都应该实现此 trait
pub trait KafkaProducerConfig: Send + Sync {
    /// Kafka Bootstrap Servers 地址
    fn kafka_bootstrap(&self) -> &str;
    
    /// 消息超时时间（毫秒），默认 5000
    fn message_timeout_ms(&self) -> u64 {
        5000
    }
    
    /// 是否启用幂等性，默认 true（推荐）
    fn enable_idempotence(&self) -> bool {
        true
    }
    
    /// 压缩类型，默认 "snappy"
    /// 可选值: "none", "gzip", "snappy", "lz4", "zstd"
    fn compression_type(&self) -> &str {
        "snappy"
    }
    
    /// 批量发送大小（字节），默认 64KB
    fn batch_size(&self) -> usize {
        64 * 1024
    }
    
    /// 批量发送延迟（毫秒），默认 10ms
    fn linger_ms(&self) -> u64 {
        10
    }
    
    /// 重试次数，默认 3
    fn retries(&self) -> u32 {
        3
    }
    
    /// 重试间隔（毫秒），默认 100ms
    fn retry_backoff_ms(&self) -> u64 {
        100
    }
    
    /// 元数据最大年龄（毫秒），默认 5 分钟
    fn metadata_max_age_ms(&self) -> u64 {
        300000
    }
}

