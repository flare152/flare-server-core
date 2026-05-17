use std::collections::HashMap;

/// Kafka producer config used by the generic MQ producer abstraction.
pub trait KafkaProducerConfig: Send + Sync {
    fn kafka_brokers(&self) -> Vec<String>;

    fn kafka_client_id(&self) -> &str {
        "flare-im"
    }

    fn kafka_acks(&self) -> &str {
        "all"
    }

    fn kafka_compression(&self) -> &str {
        "none"
    }

    fn kafka_linger_ms(&self) -> u64 {
        5
    }

    fn kafka_batch_size_bytes(&self) -> usize {
        1024 * 1024
    }

    fn kafka_message_timeout_ms(&self) -> u64 {
        30_000
    }

    fn kafka_request_timeout_ms(&self) -> u64 {
        10_000
    }

    fn kafka_enable_idempotence(&self) -> bool {
        true
    }

    fn kafka_max_in_flight_requests_per_connection(&self) -> u32 {
        5
    }

    fn kafka_options(&self) -> HashMap<String, String> {
        HashMap::new()
    }
}

/// Kafka consumer config used by the generic MQ consumer runtime.
pub trait KafkaConsumerConfig: KafkaProducerConfig {
    fn kafka_consumer_group(&self) -> &str;

    fn kafka_enable_auto_commit(&self) -> bool {
        false
    }

    fn kafka_auto_offset_reset(&self) -> &str {
        "earliest"
    }
}
