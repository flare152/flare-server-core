use std::collections::HashMap;
use std::time::Duration;

use rdkafka::ClientConfig;
use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};

use super::config::KafkaProducerConfig;
use crate::mq::producer::{Producer, ProducerConfig, ProducerError, ProducerMessage};

/// Apache Kafka producer implementation for the common MQ [`Producer`] trait.
pub struct KafkaProducer {
    producer: FutureProducer,
    timeout: Duration,
}

impl KafkaProducer {
    pub fn new<C>(config: &C) -> Result<Self, ProducerError>
    where
        C: KafkaProducerConfig + Send + Sync,
    {
        let brokers = config.kafka_brokers();
        if brokers.is_empty() {
            return Err(ProducerError::Configuration(
                "kafka brokers is empty".to_string(),
            ));
        }

        let mut client = ClientConfig::new();
        client
            .set("bootstrap.servers", brokers.join(","))
            .set("client.id", config.kafka_client_id())
            .set("acks", config.kafka_acks())
            .set("compression.type", config.kafka_compression())
            .set("linger.ms", config.kafka_linger_ms().to_string())
            .set("batch.size", config.kafka_batch_size_bytes().to_string())
            .set(
                "message.timeout.ms",
                config.kafka_message_timeout_ms().to_string(),
            )
            .set(
                "request.timeout.ms",
                config.kafka_request_timeout_ms().to_string(),
            )
            .set(
                "enable.idempotence",
                if config.kafka_enable_idempotence() {
                    "true"
                } else {
                    "false"
                },
            )
            .set(
                "max.in.flight.requests.per.connection",
                config
                    .kafka_max_in_flight_requests_per_connection()
                    .to_string(),
            );

        for (key, value) in config.kafka_options() {
            client.set(key, value);
        }

        let producer = client
            .create::<FutureProducer>()
            .map_err(|e| ProducerError::Configuration(e.to_string()))?;

        Ok(Self {
            producer,
            timeout: Duration::from_millis(config.kafka_message_timeout_ms().max(1)),
        })
    }

    fn build_headers(
        ctx: &flare_core_base::context::Ctx,
        key: Option<&str>,
        headers: Option<HashMap<String, String>>,
    ) -> OwnedHeaders {
        let mut merged = headers.unwrap_or_default();
        crate::mq::context::merge_ctx_to_headers(&mut merged, ctx);
        if let Some(key) = key.filter(|v| !v.is_empty()) {
            merged
                .entry("x-message-key".to_string())
                .or_insert_with(|| key.to_string());
        }
        merged
            .entry("content-type".to_string())
            .or_insert_with(|| "application/protobuf".to_string());

        let mut out = OwnedHeaders::new();
        for (key, value) in merged {
            out = out.insert(Header {
                key: key.as_str(),
                value: Some(value.as_bytes()),
            });
        }
        out
    }
}

#[async_trait::async_trait]
impl Producer for KafkaProducer {
    async fn send(
        &self,
        ctx: &flare_core_base::context::Ctx,
        topic: &str,
        key: Option<&str>,
        payload: Vec<u8>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), ProducerError> {
        let headers = Self::build_headers(ctx, key, headers);
        let mut record = FutureRecord::to(topic).payload(&payload).headers(headers);
        if let Some(key) = key {
            record = record.key(key);
        }

        self.producer
            .send(record, self.timeout)
            .await
            .map(|_| ())
            .map_err(|(e, _)| ProducerError::Send(e.to_string()))
    }

    async fn send_batch(
        &self,
        ctx: &flare_core_base::context::Ctx,
        messages: Vec<ProducerMessage>,
    ) -> Result<(), ProducerError> {
        for message in messages {
            self.send(
                ctx,
                &message.topic,
                message.key.as_deref(),
                message.payload,
                Some(message.headers),
            )
            .await?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "kafka-producer"
    }
}

pub struct KafkaProducerBuilder {
    _config: ProducerConfig,
}

impl KafkaProducerBuilder {
    pub fn new() -> Self {
        Self {
            _config: ProducerConfig::default(),
        }
    }

    pub fn with_config(mut self, config: ProducerConfig) -> Self {
        self._config = config;
        self
    }

    pub fn build<C>(self, config: &C) -> Result<KafkaProducer, ProducerError>
    where
        C: KafkaProducerConfig + Send + Sync,
    {
        KafkaProducer::new(config)
    }
}

impl Default for KafkaProducerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
