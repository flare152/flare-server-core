//! Kafka 生产者构建器
//!
//! 提供统一的 Kafka 生产者构建逻辑，支持所有服务复用

#[cfg(feature = "kafka")]
use rdkafka::config::ClientConfig;
#[cfg(feature = "kafka")]
use rdkafka::producer::FutureProducer;
use tracing::info;

use crate::kafka::producer_config::KafkaProducerConfig;

/// 构建 Kafka 生产者
///
/// # 参数
/// * `config` - 实现了 `KafkaProducerConfig` trait 的配置对象
///
/// # 返回
/// * `Result<FutureProducer>` - 构建好的生产者
#[cfg(feature = "kafka")]
pub fn build_kafka_producer(
    config: &dyn KafkaProducerConfig,
) -> Result<FutureProducer, rdkafka::error::KafkaError> {
    // 构建配置（链式调用，一次性构建到 create）
    // 注意：rdkafka 中某些配置项名称可能与 Kafka 官方文档不同
    // max.request.size 在 rdkafka 中可能不支持，移除该配置项
    let producer = if config.enable_idempotence() {
        // 启用幂等性时，需要设置 acks=all（确保消息不丢失）
        ClientConfig::new()
            .set("bootstrap.servers", config.kafka_bootstrap())
            .set("message.timeout.ms", &config.message_timeout_ms().to_string())
            .set("enable.idempotence", &config.enable_idempotence().to_string())
            .set("acks", "all")
            .set("compression.type", config.compression_type())
            .set("batch.size", &config.batch_size().to_string())
            .set("linger.ms", &config.linger_ms().to_string())
            .set("retries", &config.retries().to_string())
            .set("retry.backoff.ms", &config.retry_backoff_ms().to_string())
            .set("metadata.max.age.ms", &config.metadata_max_age_ms().to_string())
            .set("security.protocol", "plaintext")
            .create()?
    } else {
        ClientConfig::new()
            .set("bootstrap.servers", config.kafka_bootstrap())
            .set("message.timeout.ms", &config.message_timeout_ms().to_string())
            .set("enable.idempotence", &config.enable_idempotence().to_string())
            .set("compression.type", config.compression_type())
            .set("batch.size", &config.batch_size().to_string())
            .set("linger.ms", &config.linger_ms().to_string())
            .set("retries", &config.retries().to_string())
            .set("retry.backoff.ms", &config.retry_backoff_ms().to_string())
            .set("metadata.max.age.ms", &config.metadata_max_age_ms().to_string())
            .set("security.protocol", "plaintext")
            .create()?
    };
    
    info!(
        bootstrap = %config.kafka_bootstrap(),
        timeout_ms = config.message_timeout_ms(),
        idempotence = config.enable_idempotence(),
        compression = %config.compression_type(),
        "Kafka producer created successfully"
    );
    
    Ok(producer)
}
