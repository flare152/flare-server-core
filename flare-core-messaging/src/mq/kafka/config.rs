//! Kafka 配置 Trait 和默认实现

use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::producer::FutureProducer;

// ========== 生产者配置 ==========

/// Kafka 生产者配置 Trait
pub trait KafkaProducerConfig: Send + Sync {
    /// Kafka bootstrap servers
    fn kafka_bootstrap(&self) -> &str;

    /// 消息超时时间（毫秒）
    fn message_timeout_ms(&self) -> u64;

    /// 是否启用幂等性
    fn enable_idempotence(&self) -> bool;

    /// 压缩类型
    fn compression_type(&self) -> &str;

    /// 批处理大小（字节）
    fn batch_size(&self) -> usize;

    /// 延迟时间（毫秒）
    fn linger_ms(&self) -> u64;

    /// 重试次数
    fn retries(&self) -> u32;

    /// 重试退避时间（毫秒）
    fn retry_backoff_ms(&self) -> u64;

    /// 元数据最大存活时间（毫秒）
    fn metadata_max_age_ms(&self) -> u64;
}

// ========== 消费者配置 ==========

/// Kafka 消费者配置 Trait
pub trait KafkaConsumerConfig: Send + Sync {
    /// Kafka bootstrap servers
    fn kafka_bootstrap(&self) -> &str;

    /// Consumer group ID
    fn consumer_group(&self) -> &str;

    /// 是否启用自动提交
    fn enable_auto_commit(&self) -> bool;

    /// 会话超时时间（毫秒）
    fn session_timeout_ms(&self) -> u64;

    /// 自动 offset 重置策略
    fn auto_offset_reset(&self) -> &str;

    /// 最小抓取字节数
    fn fetch_min_bytes(&self) -> usize;

    /// 最大抓取等待时间（毫秒）
    fn fetch_max_wait_ms(&self) -> u64;

    /// 最大抓取消息字节数
    fn fetch_message_max_bytes(&self) -> usize;

    /// 最大分区抓取字节数
    fn max_partition_fetch_bytes(&self) -> usize;

    /// 元数据最大存活时间（毫秒）
    fn metadata_max_age_ms(&self) -> u64;
}

/// 在保留其余 Kafka 参数的前提下，覆盖 `group.id`（用于 [super::consumer::KafkaMessageFetcher::new_with_consumer_group]）。
#[derive(Debug, Clone, Copy)]
pub struct KafkaConsumerGroupOverride<'a, C: KafkaConsumerConfig> {
    pub inner: &'a C,
    pub group_id: &'a str,
}

impl<C: KafkaConsumerConfig> KafkaConsumerConfig for KafkaConsumerGroupOverride<'_, C> {
    fn kafka_bootstrap(&self) -> &str {
        self.inner.kafka_bootstrap()
    }

    fn consumer_group(&self) -> &str {
        self.group_id
    }

    fn enable_auto_commit(&self) -> bool {
        self.inner.enable_auto_commit()
    }

    fn session_timeout_ms(&self) -> u64 {
        self.inner.session_timeout_ms()
    }

    fn auto_offset_reset(&self) -> &str {
        self.inner.auto_offset_reset()
    }

    fn fetch_min_bytes(&self) -> usize {
        self.inner.fetch_min_bytes()
    }

    fn fetch_max_wait_ms(&self) -> u64 {
        self.inner.fetch_max_wait_ms()
    }

    fn fetch_message_max_bytes(&self) -> usize {
        self.inner.fetch_message_max_bytes()
    }

    fn max_partition_fetch_bytes(&self) -> usize {
        self.inner.max_partition_fetch_bytes()
    }

    fn metadata_max_age_ms(&self) -> u64 {
        self.inner.metadata_max_age_ms()
    }
}

// ========== 构建函数 ==========

/// 构建 Kafka 生产者
pub fn build_kafka_producer<C>(config: &C) -> Result<FutureProducer, Box<dyn std::error::Error>>
where
    C: KafkaProducerConfig,
{
    ClientConfig::new()
        .set("bootstrap.servers", config.kafka_bootstrap())
        .set(
            "message.timeout.ms",
            &config.message_timeout_ms().to_string(),
        )
        .set(
            "enable.idempotence",
            if config.enable_idempotence() {
                "true"
            } else {
                "false"
            },
        )
        .set("compression.type", config.compression_type())
        .set("batch.size", &config.batch_size().to_string())
        .set("linger.ms", &config.linger_ms().to_string())
        .set("retries", &config.retries().to_string())
        .set("retry.backoff.ms", &config.retry_backoff_ms().to_string())
        .set(
            "metadata.max.age.ms",
            &config.metadata_max_age_ms().to_string(),
        )
        .set("acks", "all")
        .set(
            "delivery.timeout.ms",
            &(config.message_timeout_ms() * 2).to_string(),
        )
        .create()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// 构建 Kafka 消费者
pub fn build_kafka_consumer<C>(config: &C) -> Result<StreamConsumer, Box<dyn std::error::Error>>
where
    C: KafkaConsumerConfig,
{
    ClientConfig::new()
        .set("bootstrap.servers", config.kafka_bootstrap())
        .set("group.id", config.consumer_group())
        .set(
            "enable.auto.commit",
            if config.enable_auto_commit() {
                "true"
            } else {
                "false"
            },
        )
        .set("auto.offset.reset", config.auto_offset_reset())
        .set(
            "session.timeout.ms",
            &config.session_timeout_ms().to_string(),
        )
        .set("fetch.min.bytes", &config.fetch_min_bytes().to_string())
        .set("fetch.wait.max.ms", &config.fetch_max_wait_ms().to_string())
        .set(
            "fetch.message.max.bytes",
            &config.fetch_message_max_bytes().to_string(),
        )
        .set(
            "max.partition.fetch.bytes",
            &config.max_partition_fetch_bytes().to_string(),
        )
        .set(
            "metadata.max.age.ms",
            &config.metadata_max_age_ms().to_string(),
        )
        .create()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// 订阅主题（分区分配由协调器在首次 poll 时完成）
pub async fn subscribe_and_wait_for_assignment(
    consumer: &StreamConsumer,
    topics: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    consumer.subscribe(topics)?;
    Ok(())
}
