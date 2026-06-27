use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use super::handler::MessageHandler;
use super::types::MessageResult;
use super::types::{ConsumerError, ContentType, Message};
use crate::mq::producer::Producer;

pub const HEADER_RETRY_COUNT: &str = "x-flare-retry-count";
pub const HEADER_FAILURE_REASON: &str = "x-flare-failure-reason";
pub const HEADER_FAILURE_ERROR: &str = "x-flare-failure-error";
pub const HEADER_ORIGINAL_TOPIC: &str = "x-flare-original-topic";
pub const HEADER_ORIGINAL_MESSAGE_ID: &str = "x-flare-original-message-id";
pub const HEADER_ORIGINAL_PARTITION: &str = "x-flare-original-partition";
pub const HEADER_ORIGINAL_OFFSET: &str = "x-flare-original-offset";
pub const HEADER_RETRY_NOT_BEFORE_UNIX_MS: &str = "x-flare-retry-not-before-unix-ms";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    Retry,
    DeadLetter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureContext {
    pub reason: String,
    pub error: Option<String>,
    pub retry_count: u32,
}

impl FailureContext {
    pub fn new(
        reason: impl Into<String>,
        error: Option<impl Into<String>>,
        retry_count: u32,
    ) -> Self {
        Self {
            reason: reason.into(),
            error: error.map(Into::into),
            retry_count,
        }
    }

    pub fn from_error(reason: &'static str, error: &ConsumerError, retry_count: u32) -> Self {
        Self::new(reason, Some(error.to_string()), retry_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub enable_dlq: bool,
}

impl RetryPolicy {
    pub fn new(max_retries: u32, enable_dlq: bool) -> Self {
        Self {
            max_retries,
            enable_dlq,
        }
    }

    pub fn action_for_nack(&self, retry_count: u32) -> FailureAction {
        self.retry_or_dead_letter(retry_count)
    }

    pub fn action_for_error(&self, error: &ConsumerError, retry_count: u32) -> FailureAction {
        if error.is_retryable() {
            self.retry_or_dead_letter(retry_count)
        } else if self.enable_dlq {
            FailureAction::DeadLetter
        } else {
            FailureAction::Retry
        }
    }

    pub fn action_for_dead_letter(&self) -> FailureAction {
        if self.enable_dlq {
            FailureAction::DeadLetter
        } else {
            FailureAction::Retry
        }
    }

    fn retry_or_dead_letter(&self, retry_count: u32) -> FailureAction {
        if retry_count < self.max_retries {
            FailureAction::Retry
        } else if self.enable_dlq {
            FailureAction::DeadLetter
        } else {
            FailureAction::Retry
        }
    }
}

impl From<&super::runtime::ConsumerConfig> for RetryPolicy {
    fn from(config: &super::runtime::ConsumerConfig) -> Self {
        Self::new(config.max_retries, config.enable_dlq)
    }
}

#[async_trait]
pub trait RetryPublisher: Send + Sync {
    async fn publish_retry(
        &self,
        message: &Message,
        failure: FailureContext,
    ) -> Result<(), ConsumerError>;
}

#[async_trait]
pub trait DeadLetterPublisher: Send + Sync {
    async fn publish_dead_letter(
        &self,
        message: &Message,
        failure: FailureContext,
    ) -> Result<(), ConsumerError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureTopic {
    Original,
    Fixed(String),
    Suffix(String),
}

impl FailureTopic {
    pub fn original() -> Self {
        Self::Original
    }

    pub fn fixed(topic: impl Into<String>) -> Self {
        Self::Fixed(topic.into())
    }

    pub fn suffix(suffix: impl Into<String>) -> Self {
        Self::Suffix(suffix.into())
    }

    pub fn resolve(&self, original_topic: &str) -> String {
        match self {
            Self::Original => original_topic.to_string(),
            Self::Fixed(topic) => topic.clone(),
            Self::Suffix(suffix) => format!("{original_topic}{suffix}"),
        }
    }
}

pub struct ProducerRetryPublisher {
    producer: Arc<dyn Producer>,
    topic: FailureTopic,
    not_before_delay: Option<Duration>,
}

impl ProducerRetryPublisher {
    pub fn new(producer: Arc<dyn Producer>, topic: FailureTopic) -> Self {
        Self {
            producer,
            topic,
            not_before_delay: None,
        }
    }

    pub fn with_not_before_delay(mut self, delay: Duration) -> Self {
        self.not_before_delay = Some(delay);
        self
    }
}

#[async_trait]
impl RetryPublisher for ProducerRetryPublisher {
    async fn publish_retry(
        &self,
        message: &Message,
        failure: FailureContext,
    ) -> Result<(), ConsumerError> {
        let topic = self.topic.resolve(&message.context.topic);
        let mut headers = failure_headers(message, &failure, failure.retry_count.saturating_add(1));
        if let Some(delay) = self.not_before_delay {
            headers.insert(
                HEADER_RETRY_NOT_BEFORE_UNIX_MS.to_string(),
                unix_epoch_millis()
                    .saturating_add(delay.as_millis() as u64)
                    .to_string(),
            );
        }
        self.producer
            .send(
                &message.context.ctx,
                &topic,
                message.context.key.as_deref(),
                message.payload.clone(),
                Some(headers),
            )
            .await
            .map_err(|err| ConsumerError::Retry(err.to_string()))
    }
}

pub struct ProducerDeadLetterPublisher {
    producer: Arc<dyn Producer>,
    topic: FailureTopic,
}

impl ProducerDeadLetterPublisher {
    pub fn new(producer: Arc<dyn Producer>, topic: FailureTopic) -> Self {
        Self { producer, topic }
    }
}

#[async_trait]
impl DeadLetterPublisher for ProducerDeadLetterPublisher {
    async fn publish_dead_letter(
        &self,
        message: &Message,
        failure: FailureContext,
    ) -> Result<(), ConsumerError> {
        let topic = self.topic.resolve(&message.context.topic);
        let headers = failure_headers(message, &failure, failure.retry_count);
        self.producer
            .send(
                &message.context.ctx,
                &topic,
                message.context.key.as_deref(),
                message.payload.clone(),
                Some(headers),
            )
            .await
            .map_err(|err| ConsumerError::DeadLetter(err.to_string()))
    }
}

pub struct RetryForwarderHandler {
    producer: Arc<dyn Producer>,
    name: String,
}

impl RetryForwarderHandler {
    pub fn new(producer: Arc<dyn Producer>) -> Self {
        Self {
            producer,
            name: "mq-retry-forwarder".to_string(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    async fn wait_until_not_before(message: &Message) {
        let Some(not_before) = message
            .context
            .headers
            .get(HEADER_RETRY_NOT_BEFORE_UNIX_MS)
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return;
        };

        let now = unix_epoch_millis();
        if not_before > now {
            tokio::time::sleep(Duration::from_millis(not_before - now)).await;
        }
    }
}

#[async_trait]
impl MessageHandler for RetryForwarderHandler {
    async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError> {
        let Some(original_topic) = message
            .context
            .headers
            .get(HEADER_ORIGINAL_TOPIC)
            .filter(|topic| !topic.trim().is_empty())
            .cloned()
        else {
            return Err(ConsumerError::DeadLetter(format!(
                "retry message {} missing {HEADER_ORIGINAL_TOPIC}",
                message.context.message_id
            )));
        };

        Self::wait_until_not_before(&message).await;

        let mut headers = message.context.headers.clone();
        headers.remove(HEADER_RETRY_NOT_BEFORE_UNIX_MS);
        self.producer
            .send(
                &message.context.ctx,
                &original_topic,
                message.context.key.as_deref(),
                message.payload,
                Some(headers),
            )
            .await
            .map_err(|err| ConsumerError::Retry(err.to_string()))?;

        tracing::trace!(
            original_topic = %original_topic,
            retry_topic = %message.context.topic,
            message_id = %message.context.message_id,
            retry_count = message.context.retry_count,
            "Retry message forwarded to original topic"
        );

        Ok(MessageResult::Ack)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Default)]
pub struct ConsumerFailurePublishers {
    pub retry: Option<Arc<dyn RetryPublisher>>,
    pub dead_letter: Option<Arc<dyn DeadLetterPublisher>>,
}

impl ConsumerFailurePublishers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_retry(mut self, retry: Arc<dyn RetryPublisher>) -> Self {
        self.retry = Some(retry);
        self
    }

    pub fn with_dead_letter(mut self, dead_letter: Arc<dyn DeadLetterPublisher>) -> Self {
        self.dead_letter = Some(dead_letter);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.retry.is_none() && self.dead_letter.is_none()
    }
}

fn unix_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn retry_count_from_headers(headers: &HashMap<String, String>) -> u32 {
    headers
        .get(HEADER_RETRY_COUNT)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0)
}

fn failure_headers(
    message: &Message,
    failure: &FailureContext,
    retry_count: u32,
) -> HashMap<String, String> {
    let mut headers = message.context.headers.clone();
    headers.insert(HEADER_RETRY_COUNT.to_string(), retry_count.to_string());
    headers.insert(HEADER_FAILURE_REASON.to_string(), failure.reason.clone());
    headers.insert(
        HEADER_ORIGINAL_TOPIC.to_string(),
        message.context.topic.clone(),
    );
    headers.insert(
        HEADER_ORIGINAL_MESSAGE_ID.to_string(),
        message.context.message_id.clone(),
    );
    headers.insert(
        HEADER_ORIGINAL_PARTITION.to_string(),
        message.context.partition.to_string(),
    );
    headers.insert(
        HEADER_ORIGINAL_OFFSET.to_string(),
        message.context.offset.to_string(),
    );
    if let Some(error) = &failure.error {
        headers.insert(HEADER_FAILURE_ERROR.to_string(), error.clone());
    }
    headers
        .entry("content-type".to_string())
        .or_insert_with(|| content_type_header(&message.content_type).to_string());
    headers
}

fn content_type_header(content_type: &ContentType) -> &'static str {
    match content_type {
        ContentType::Json => "application/json",
        ContentType::Protobuf => "application/protobuf",
        ContentType::Avro => "application/avro",
        ContentType::Raw => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mq::consumer::types::{Message, MessageContext};
    use crate::mq::producer::{ProducerError, ProducerMessage};
    use flare_core_base::context::Context;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingProducer {
        sends: Mutex<Vec<RecordedSend>>,
    }

    #[derive(Debug, Clone)]
    struct RecordedSend {
        topic: String,
        key: Option<String>,
        payload: Vec<u8>,
        headers: HashMap<String, String>,
    }

    #[async_trait]
    impl Producer for RecordingProducer {
        async fn send(
            &self,
            _ctx: &flare_core_base::context::Ctx,
            topic: &str,
            key: Option<&str>,
            payload: Vec<u8>,
            headers: Option<HashMap<String, String>>,
        ) -> Result<(), ProducerError> {
            self.sends.lock().expect("lock sends").push(RecordedSend {
                topic: topic.to_string(),
                key: key.map(ToString::to_string),
                payload,
                headers: headers.unwrap_or_default(),
            });
            Ok(())
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
            "recording-producer"
        }
    }

    #[test]
    fn retry_policy_moves_retryable_failure_to_dlq_after_limit() {
        let policy = RetryPolicy::new(2, true);
        let error = ConsumerError::Handler("temporary failure".to_string());

        assert_eq!(policy.action_for_error(&error, 0), FailureAction::Retry);
        assert_eq!(policy.action_for_error(&error, 1), FailureAction::Retry);
        assert_eq!(
            policy.action_for_error(&error, 2),
            FailureAction::DeadLetter
        );
    }

    #[test]
    fn failure_headers_preserve_original_identity_and_increment_retry() {
        let ctx = Arc::new(Context::with_request_id("req-1"));
        let mut message_context = MessageContext::new(ctx, "flare.test".to_string());
        message_context.message_id = "msg-1".to_string();
        message_context.partition = 3;
        message_context.offset = 42;
        let message = Message::new(vec![1, 2, 3], ContentType::Raw, message_context);
        let failure = FailureContext::new("handler_nack", None::<String>, 1);

        let headers = failure_headers(&message, &failure, 2);

        assert_eq!(
            headers.get(HEADER_RETRY_COUNT).map(String::as_str),
            Some("2")
        );
        assert_eq!(
            headers.get(HEADER_ORIGINAL_TOPIC).map(String::as_str),
            Some("flare.test")
        );
        assert_eq!(
            headers.get(HEADER_ORIGINAL_MESSAGE_ID).map(String::as_str),
            Some("msg-1")
        );
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/octet-stream")
        );
    }

    #[tokio::test]
    async fn retry_publisher_writes_retry_topic_and_not_before_header() {
        let producer = Arc::new(RecordingProducer::default());
        let publisher =
            ProducerRetryPublisher::new(producer.clone(), FailureTopic::fixed("flare.retry.5s"))
                .with_not_before_delay(Duration::from_millis(1));

        let ctx = Arc::new(Context::with_request_id("req-2"));
        let mut message_context = MessageContext::new(ctx, "flare.main".to_string());
        message_context.message_id = "msg-2".to_string();
        message_context.key = Some("k1".to_string());
        let message = Message::new(vec![4, 5, 6], ContentType::Raw, message_context);

        publisher
            .publish_retry(&message, FailureContext::new("nack", None::<String>, 0))
            .await
            .expect("retry publish succeeds");

        let sends = producer.sends.lock().expect("lock sends");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].topic, "flare.retry.5s");
        assert_eq!(sends[0].key.as_deref(), Some("k1"));
        assert_eq!(sends[0].payload, vec![4, 5, 6]);
        assert_eq!(
            sends[0].headers.get(HEADER_RETRY_COUNT).map(String::as_str),
            Some("1")
        );
        assert_eq!(
            sends[0]
                .headers
                .get(HEADER_ORIGINAL_TOPIC)
                .map(String::as_str),
            Some("flare.main")
        );
        assert!(
            sends[0]
                .headers
                .contains_key(HEADER_RETRY_NOT_BEFORE_UNIX_MS)
        );
    }

    #[tokio::test]
    async fn retry_forwarder_publishes_to_original_topic() {
        let producer = Arc::new(RecordingProducer::default());
        let handler = RetryForwarderHandler::new(producer.clone());
        let ctx = Arc::new(Context::with_request_id("req-3"));
        let mut message_context = MessageContext::new(ctx, "flare.retry.5s".to_string());
        message_context.message_id = "retry-msg".to_string();
        message_context.key = Some("k2".to_string());
        message_context
            .headers
            .insert(HEADER_ORIGINAL_TOPIC.to_string(), "flare.main".to_string());
        message_context.headers.insert(
            HEADER_RETRY_NOT_BEFORE_UNIX_MS.to_string(),
            unix_epoch_millis().to_string(),
        );
        let message = Message::new(vec![7, 8, 9], ContentType::Raw, message_context);

        let result = handler.handle(message).await.expect("forward succeeds");

        assert_eq!(result, MessageResult::Ack);
        let sends = producer.sends.lock().expect("lock sends");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].topic, "flare.main");
        assert_eq!(sends[0].key.as_deref(), Some("k2"));
        assert_eq!(sends[0].payload, vec![7, 8, 9]);
        assert!(
            !sends[0]
                .headers
                .contains_key(HEADER_RETRY_NOT_BEFORE_UNIX_MS)
        );
    }
}
