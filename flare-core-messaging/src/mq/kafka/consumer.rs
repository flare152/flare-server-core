use std::collections::HashMap;
use std::sync::Arc;

use rdkafka::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{BorrowedMessage, Headers, Message as KafkaMessage};
use rdkafka::{Offset, TopicPartitionList};

use super::config::KafkaConsumerConfig;
use crate::mq::consumer::dispatcher::Dispatcher;
use crate::mq::consumer::{
    ConsumerConfig, ConsumerError, ConsumerRuntimeTask, ContentType, Message, MessageAck,
    MessageContext, MessageFetcher, MqConsumerTask,
};

pub struct KafkaMessageFetcher {
    consumer: Arc<StreamConsumer>,
}

impl KafkaMessageFetcher {
    pub fn new<C>(config: &C, topics: Vec<String>) -> Result<Self, ConsumerError>
    where
        C: KafkaConsumerConfig + Send + Sync,
    {
        if topics.is_empty() {
            return Err(ConsumerError::Configuration(
                "Kafka consumer topics is empty".to_string(),
            ));
        }
        let brokers = config.kafka_brokers();
        if brokers.is_empty() {
            return Err(ConsumerError::Configuration(
                "Kafka brokers is empty".to_string(),
            ));
        }

        let mut client = ClientConfig::new();
        client
            .set("bootstrap.servers", brokers.join(","))
            .set("group.id", config.kafka_consumer_group())
            .set("client.id", config.kafka_client_id())
            .set(
                "enable.auto.commit",
                if config.kafka_enable_auto_commit() {
                    "true"
                } else {
                    "false"
                },
            )
            .set("auto.offset.reset", config.kafka_auto_offset_reset())
            .set("enable.partition.eof", "false")
            .set("session.timeout.ms", "10000")
            .set("max.poll.interval.ms", "300000");

        for (key, value) in config.kafka_options() {
            client.set(key, value);
        }

        let consumer = client
            .create::<StreamConsumer>()
            .map_err(|e| ConsumerError::Configuration(e.to_string()))?;
        let topic_refs = topics.iter().map(String::as_str).collect::<Vec<_>>();
        consumer
            .subscribe(&topic_refs)
            .map_err(|e| ConsumerError::Configuration(e.to_string()))?;

        tracing::info!(
            consumer_group = %config.kafka_consumer_group(),
            topics = ?topics,
            "Subscribed to Kafka topics"
        );

        Ok(Self {
            consumer: Arc::new(consumer),
        })
    }

    fn decode_message(&self, msg: &BorrowedMessage<'_>) -> Result<Message, ConsumerError> {
        let payload = msg.payload().unwrap_or_default().to_vec();
        let topic = msg.topic().to_string();

        let mut headers = HashMap::new();
        if let Some(kafka_headers) = msg.headers() {
            for header in kafka_headers.iter() {
                if let Some(value) = header.value {
                    headers.insert(
                        header.key.to_string(),
                        String::from_utf8_lossy(value).to_string(),
                    );
                }
            }
        }

        let key = msg
            .key()
            .map(|v| String::from_utf8_lossy(v).to_string())
            .or_else(|| headers.get("x-message-key").cloned());
        let message_id = headers
            .get("x-message-id")
            .cloned()
            .unwrap_or_else(|| format!("{}:{}:{}", topic, msg.partition(), msg.offset()));
        let ctx = crate::mq::context::mq_headers_to_ctx(&headers);
        let content_type = headers
            .get("content-type")
            .and_then(|v| ContentType::from_str(v))
            .unwrap_or(ContentType::Protobuf);

        let context = MessageContext {
            ctx,
            message_id,
            topic: topic.clone(),
            partition: msg.partition(),
            offset: msg.offset(),
            key,
            headers,
            started_at: std::time::Instant::now(),
            retry_count: 0,
            metadata: HashMap::new(),
        };

        Ok(
            Message::new(payload, content_type, context).with_ack_handle(Arc::new(
                KafkaMessageAck {
                    consumer: self.consumer.clone(),
                    topic,
                    partition: msg.partition(),
                    offset: msg.offset(),
                },
            )),
        )
    }
}

#[async_trait::async_trait]
impl MessageFetcher for KafkaMessageFetcher {
    async fn fetch(&mut self) -> Result<Option<Message>, ConsumerError> {
        match self.consumer.recv().await {
            Ok(msg) => self.decode_message(&msg).map(Some),
            Err(e) => Err(ConsumerError::Connection(e.to_string())),
        }
    }
}

struct KafkaMessageAck {
    consumer: Arc<StreamConsumer>,
    topic: String,
    partition: i32,
    offset: i64,
}

#[async_trait::async_trait]
impl MessageAck for KafkaMessageAck {
    async fn ack(&self) -> Result<(), ConsumerError> {
        let mut tpl = TopicPartitionList::new();
        tpl.add_partition_offset(
            &self.topic,
            self.partition,
            Offset::Offset(self.offset.saturating_add(1)),
        )
        .map_err(|e| ConsumerError::Connection(e.to_string()))?;
        self.consumer
            .commit(&tpl, CommitMode::Async)
            .map_err(|e| ConsumerError::Connection(e.to_string()))
    }

    async fn nack(&self) -> Result<(), ConsumerError> {
        Ok(())
    }

    async fn term(&self) -> Result<(), ConsumerError> {
        self.ack().await
    }
}

pub fn build_kafka_consumer_tasks<C>(
    config: &C,
    consumer_config: ConsumerConfig,
    dispatcher: Arc<dyn Dispatcher>,
    task_name_prefix: impl AsRef<str>,
) -> Result<Vec<MqConsumerTask>, ConsumerError>
where
    C: KafkaConsumerConfig + Send + Sync,
{
    let mut topics = dispatcher.topics();
    if topics.is_empty() {
        return Err(ConsumerError::Configuration(
            "Kafka consumer dispatcher has no registered topics".to_string(),
        ));
    }
    topics.sort();
    topics.dedup();

    let fetcher = KafkaMessageFetcher::new(config, topics)?;
    let consumer = ConsumerRuntimeTask::from_parts(consumer_config, dispatcher, fetcher);
    Ok(vec![MqConsumerTask::new(
        task_name_prefix.as_ref().to_string(),
        Box::new(consumer),
    )])
}
