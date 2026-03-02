#[cfg(feature = "kafka")]
use rdkafka::config::ClientConfig;
#[cfg(feature = "kafka")]
use rdkafka::consumer::{Consumer, StreamConsumer};
#[cfg(feature = "kafka")]
use rdkafka::TopicPartitionList;
use tracing::{debug, info, warn};

use super::consumer_config::KafkaConsumerConfig;

#[cfg(feature = "kafka")]
pub fn build_kafka_consumer(
    config: &dyn KafkaConsumerConfig,
) -> Result<StreamConsumer, rdkafka::error::KafkaError> {
    ClientConfig::new()
        .set("bootstrap.servers", config.kafka_bootstrap())
        .set("group.id", config.consumer_group())
        .set("enable.partition.eof", "false")
        .set("session.timeout.ms", &config.session_timeout_ms().to_string())
        .set("enable.auto.commit", &config.enable_auto_commit().to_string())
        .set("auto.offset.reset", config.auto_offset_reset())
        .set("security.protocol", "plaintext")
        .set("fetch.message.max.bytes", &config.fetch_message_max_bytes().to_string())
        .set("max.partition.fetch.bytes", &config.max_partition_fetch_bytes().to_string())
        .set("fetch.min.bytes", &config.fetch_min_bytes().to_string())
        .set("fetch.wait.max.ms", &config.fetch_max_wait_ms().to_string())
        .set("metadata.max.age.ms", &config.metadata_max_age_ms().to_string())
        .create()
}

#[cfg(feature = "kafka")]
pub async fn subscribe_and_wait_for_assignment(
    consumer: &StreamConsumer,
    topic: &str,
    max_wait_seconds: u64,
) -> Result<(), String> {
    match consumer.subscribe(&[topic]) {
        Ok(_) => info!(topic = %topic, "Subscribed to Kafka topic"),
        Err(err) => {
            warn!(error = %err, topic = %topic, "Subscribe failed, trying manual partition assign");
            let mut tpl = TopicPartitionList::new();
            tpl.add_partition_offset(topic, 0, rdkafka::Offset::Beginning)
                .map_err(|e| format!("add_partition_offset: {}", e))?;
            consumer.assign(&tpl).map_err(|e| format!("assign: {}", e))?;
            info!(topic = %topic, partition = 0, "Manually assigned partition 0");
            return Ok(());
        }
    }
    let mut retries = 0u64;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        retries += 1;
        match consumer.assignment() {
            Ok(a) if a.count() > 0 => {
                info!(partition_count = a.count(), retries = retries, "Consumer assigned");
                return Ok(());
            }
            _ if retries >= max_wait_seconds => {
                warn!(retries = retries, topic = %topic, "Assignment timeout, manual assign");
                let mut tpl = TopicPartitionList::new();
                tpl.add_partition_offset(topic, 0, rdkafka::Offset::Beginning)
                    .map_err(|e| format!("add_partition_offset: {}", e))?;
                consumer.assign(&tpl).map_err(|e| format!("assign: {}", e))?;
                return Ok(());
            }
            _ => debug!(retries = retries, "Waiting for assignment"),
        }
    }
}
