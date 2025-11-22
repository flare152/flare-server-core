//! Kafka 消费者构建器
//!
//! 提供统一的 Kafka 消费者构建和订阅逻辑，支持所有服务复用

#[cfg(feature = "kafka")]
use rdkafka::config::ClientConfig;
#[cfg(feature = "kafka")]
use rdkafka::consumer::{Consumer, StreamConsumer};
#[cfg(feature = "kafka")]
use rdkafka::TopicPartitionList;
use tracing::{debug, error, info, warn};

use crate::kafka::consumer_config::KafkaConsumerConfig;

/// 构建 Kafka 消费者
///
/// # 参数
/// * `config` - 实现了 `KafkaConsumerConfig` trait 的配置对象
///
/// # 返回
/// * `Result<StreamConsumer>` - 构建好的消费者
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

/// 订阅 Kafka topic 并等待 partition assignment
///
/// # 参数
/// * `consumer` - Kafka 消费者
/// * `topic` - Topic 名称
/// * `max_wait_seconds` - 最大等待时间（秒）
///
/// # 返回
/// * `Result<()>` - 成功或错误
#[cfg(feature = "kafka")]
pub async fn subscribe_and_wait_for_assignment(
    consumer: &StreamConsumer,
    topic: &str,
    max_wait_seconds: u64,
) -> Result<(), String> {
    // 尝试订阅 topic
    match consumer.subscribe(&[topic]) {
        Ok(_) => {
            info!(
                topic = %topic,
                "Successfully subscribed to Kafka topic"
            );
        }
        Err(err) => {
            warn!(
                error = %err,
                topic = %topic,
                "Failed to subscribe to Kafka topic, will try manual partition assignment"
            );
            
            // Fallback: 手动分配 partition 0
            let mut tpl = TopicPartitionList::new();
            tpl.add_partition_offset(topic, 0, rdkafka::Offset::Beginning)
                .map_err(|e| format!("Failed to add partition offset: {}", e))?;
            
            consumer.assign(&tpl).map_err(|e| {
                format!("Failed to manually assign partition: {}", e)
            })?;
            
            info!(
                topic = %topic,
                partition = 0,
                "Manually assigned to partition 0 as fallback"
            );
            return Ok(()); // 手动分配后立即返回
        }
    }
    
    // 等待 partition assignment
    let mut retries = 0;
    let max_retries = max_wait_seconds;
    
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        retries += 1;
        
        match consumer.assignment() {
            Ok(assignment) => {
                if assignment.count() > 0 {
                    info!(
                        partition_count = assignment.count(),
                        retries = retries,
                        "Consumer assigned to {} partitions after {} retries",
                        assignment.count(),
                        retries
                    );
                    return Ok(());
                } else if retries >= max_retries {
                    // 超时后尝试手动分配
                    warn!(
                        retries = retries,
                        topic = %topic,
                        "Consumer still not assigned after {} retries, trying manual assignment",
                        retries
                    );
                    
                    let mut tpl = TopicPartitionList::new();
                    tpl.add_partition_offset(topic, 0, rdkafka::Offset::Beginning)
                        .map_err(|e| format!("Failed to add partition offset: {}", e))?;
                    
                    if let Err(e) = consumer.assign(&tpl) {
                        return Err(format!(
                            "Failed to manually assign partition after timeout: {}",
                            e
                        ));
                    }
                    
                    info!(
                        topic = %topic,
                        partition = 0,
                        "Successfully manually assigned to partition 0 after timeout"
                    );
                    return Ok(());
                } else {
                    debug!(
                        retries = retries,
                        "Waiting for partition assignment (attempt {}/{})",
                        retries,
                        max_retries
                    );
                }
            }
            Err(err) => {
                if retries >= max_retries {
                    return Err(format!(
                        "Failed to get consumer assignment after {} retries: {}",
                        retries, err
                    ));
                }
            }
        }
    }
}

