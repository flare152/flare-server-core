#[cfg(feature = "kafka")]
use rdkafka::config::ClientConfig;
#[cfg(feature = "kafka")]
use rdkafka::producer::FutureProducer;
use tracing::info;

use super::producer_config::KafkaProducerConfig;

#[cfg(feature = "kafka")]
pub fn build_kafka_producer(
    config: &dyn KafkaProducerConfig,
) -> Result<FutureProducer, rdkafka::error::KafkaError> {
    let producer = if config.enable_idempotence() {
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
        "Kafka producer created"
    );
    Ok(producer)
}
