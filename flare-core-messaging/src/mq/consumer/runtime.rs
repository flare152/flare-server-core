//! MQ 消费者运行时
//!
//! 负责消费循环、并发控制、优雅关闭等运行时功能。
//! 轮询/退避/并发与 [crate::runtime::config::PollWorkerConfig]、[crate::runtime::config::RuntimeConfig] 对齐。
//!
//! 与 [super::task::MqConsumer] / [crate::runtime::ServiceRuntime] 集成：使用 [ConsumerRuntimeTask]，
//! 通过 [crate::runtime::ServiceRuntime::add_mq_consumer] 或 [crate::runtime::ServiceRuntime::add_mq_consumer_runtime] 注册。

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::oneshot;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::Duration;

use flare_core_runtime::config::{PollWorkerConfig, RuntimeConfig};
use flare_core_runtime::task::TaskResult;

use super::task::MqConsumer;

use super::dispatcher::Dispatcher;
use super::failure::{
    ConsumerFailurePublishers, DeadLetterPublisher, FailureAction, FailureContext, RetryPolicy,
    RetryPublisher,
};
use super::types::{ConsumerError, Message, MessageResult};

type OrderedKeyLocks = Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>;
type BatchAckHandle = (
    String,
    String,
    u32,
    Option<Arc<dyn super::types::MessageAck>>,
    Option<Message>,
);

/// Generic consumer idempotency port.
///
/// Implementations should treat keys as already namespaced by the runtime
/// (`topic:message_id` or broker offset fallback). The default no-op store keeps
/// existing behavior for callers that have not wired Redis/DB yet.
#[async_trait::async_trait]
pub trait IdempotencyStore: Send + Sync {
    async fn is_consumed(&self, key: &str) -> Result<bool, ConsumerError>;
    async fn record_consumed(&self, key: &str) -> Result<(), ConsumerError>;
}

#[derive(Default)]
pub struct NoopIdempotencyStore;

#[async_trait::async_trait]
impl IdempotencyStore for NoopIdempotencyStore {
    async fn is_consumed(&self, _key: &str) -> Result<bool, ConsumerError> {
        Ok(false)
    }

    async fn record_consumed(&self, _key: &str) -> Result<(), ConsumerError> {
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryIdempotencyStore {
    consumed: Mutex<HashSet<String>>,
}

#[async_trait::async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn is_consumed(&self, key: &str) -> Result<bool, ConsumerError> {
        Ok(self.consumed.lock().await.contains(key))
    }

    async fn record_consumed(&self, key: &str) -> Result<(), ConsumerError> {
        self.consumed.lock().await.insert(key.to_string());
        Ok(())
    }
}

/// 消费者配置
#[derive(Debug, Clone)]
pub struct ConsumerConfig {
    /// 与 [RuntimeConfig::default_poll_worker] 同语义的轮询工作参数（并发、idle/error 退避）
    pub poll: PollWorkerConfig,
    /// 批处理大小
    pub batch_size: usize,
    /// 批处理超时（毫秒）
    pub batch_timeout_ms: u64,
    /// 是否启用顺序消费
    pub ordered: bool,
    /// 是否启用幂等性检查
    pub idempotent: bool,
    /// 最大重试次数
    pub max_retries: u32,
    /// 是否启用 DLQ
    pub enable_dlq: bool,
    /// JetStream：覆盖 `JetStreamConsumerConfig::consumer_group`（`None` 时使用传入的 JetStream 配置）
    pub consumer_group_override: Option<String>,
}

impl Default for ConsumerConfig {
    fn default() -> Self {
        Self {
            poll: PollWorkerConfig::default(),
            batch_size: 1,
            batch_timeout_ms: 1000,
            ordered: false,
            idempotent: true,
            max_retries: 3,
            enable_dlq: true,
            consumer_group_override: None,
        }
    }
}

impl ConsumerConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// 使用服务 [RuntimeConfig] 中的默认轮询参数作为 `poll`（MQ 专属字段仍为默认）
    pub fn with_runtime_poll_defaults(mut self, rt: &RuntimeConfig) -> Self {
        self.poll = rt.default_poll_worker.clone();
        self
    }

    pub fn with_poll(mut self, poll: PollWorkerConfig) -> Self {
        self.poll = poll;
        self
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.poll.concurrency = concurrency;
        self
    }

    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    pub fn with_batch_timeout_ms(mut self, batch_timeout_ms: u64) -> Self {
        self.batch_timeout_ms = batch_timeout_ms;
        self
    }

    pub fn with_ordered(mut self, ordered: bool) -> Self {
        self.ordered = ordered;
        self
    }

    pub fn with_idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = idempotent;
        self
    }

    /// JetStream：设置 `group.id` 覆盖（与 [crate::mq::jetstream::JetStreamMessageFetcher::new_with_consumer_group] 一致）
    pub fn with_consumer_group(mut self, group_id: impl Into<String>) -> Self {
        self.consumer_group_override = Some(group_id.into());
        self
    }
}

/// 消费者运行时
pub struct ConsumerRuntime {
    config: ConsumerConfig,
    dispatcher: Arc<dyn Dispatcher>,
    key_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    retry_policy: RetryPolicy,
    retry_publisher: Option<Arc<dyn RetryPublisher>>,
    dead_letter_publisher: Option<Arc<dyn DeadLetterPublisher>>,
    idempotency_store: Arc<dyn IdempotencyStore>,
}

impl ConsumerRuntime {
    /// 创建新的消费者运行时
    pub fn new(config: ConsumerConfig, dispatcher: Arc<dyn Dispatcher>) -> Self {
        let retry_policy = RetryPolicy::from(&config);
        Self {
            config,
            dispatcher,
            key_locks: Arc::new(Mutex::new(HashMap::new())),
            retry_policy,
            retry_publisher: None,
            dead_letter_publisher: None,
            idempotency_store: Arc::new(NoopIdempotencyStore),
        }
    }

    pub fn with_retry_publisher(mut self, publisher: Arc<dyn RetryPublisher>) -> Self {
        self.retry_publisher = Some(publisher);
        self
    }

    pub fn with_dead_letter_publisher(mut self, publisher: Arc<dyn DeadLetterPublisher>) -> Self {
        self.dead_letter_publisher = Some(publisher);
        self
    }

    pub fn with_idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.idempotency_store = store;
        self
    }

    pub fn with_failure_publishers(mut self, publishers: ConsumerFailurePublishers) -> Self {
        self.retry_publisher = publishers.retry;
        self.dead_letter_publisher = publishers.dead_letter;
        self
    }

    /// 持续拉取并处理消息，直到进程退出或 `fetch` 侧永久失败（与 [Self::run_with_shutdown] 相对，用于独立 JetStream/NATS 入口）
    pub async fn run<MF>(&self, message_fetcher: &mut MF) -> Result<(), ConsumerError>
    where
        MF: MessageFetcher + Send + ?Sized,
    {
        let semaphore = Arc::new(Semaphore::new(self.config.poll.concurrency));
        let mut tasks: JoinSet<Result<(), ConsumerError>> = JoinSet::new();
        let idle_backoff = self.config.poll.idle_backoff;
        let error_backoff = self.config.poll.error_backoff;
        let batch_size = self.config.batch_size.max(1);
        let batch_timeout = Duration::from_millis(self.config.batch_timeout_ms.max(1));

        loop {
            match message_fetcher.fetch().await {
                Ok(Some(message)) => {
                    if batch_size > 1 {
                        match Self::fetch_batch(message_fetcher, message, batch_size, batch_timeout)
                            .await
                        {
                            Ok(messages) => {
                                Self::spawn_batch_task(
                                    messages,
                                    &semaphore,
                                    &self.dispatcher,
                                    &self.retry_policy,
                                    &self.retry_publisher,
                                    &self.dead_letter_publisher,
                                    &self.idempotency_store,
                                    &mut tasks,
                                    self.config.ordered,
                                    self.config.idempotent,
                                    &self.key_locks,
                                );
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "Failed to fetch message batch");
                                tokio::time::sleep(error_backoff).await;
                            }
                        }
                    } else {
                        Self::spawn_message_task(
                            message,
                            &semaphore,
                            &self.dispatcher,
                            &self.retry_policy,
                            &self.retry_publisher,
                            &self.dead_letter_publisher,
                            &self.idempotency_store,
                            &mut tasks,
                            self.config.ordered,
                            self.config.idempotent,
                            &self.key_locks,
                        );
                    }
                }
                Ok(None) => {
                    tokio::time::sleep(idle_backoff).await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to fetch message");
                    tokio::time::sleep(error_backoff).await;
                }
            }
        }
    }

    /// 在 [ServiceRuntime] 关闭信号下运行（与 [MqConsumer::consume] 的 `shutdown_rx` 对齐）
    pub async fn run_with_shutdown<MF>(
        &self,
        message_fetcher: &mut MF,
        mut shutdown_rx: oneshot::Receiver<()>,
    ) -> Result<(), ConsumerError>
    where
        MF: MessageFetcher + Send + ?Sized,
    {
        let semaphore = Arc::new(Semaphore::new(self.config.poll.concurrency));
        let mut tasks: JoinSet<Result<(), ConsumerError>> = JoinSet::new();
        let idle_backoff = self.config.poll.idle_backoff;
        let error_backoff = self.config.poll.error_backoff;
        let batch_size = self.config.batch_size.max(1);
        let batch_timeout = Duration::from_millis(self.config.batch_timeout_ms.max(1));

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::info!("Shutdown signal received, stopping consumer");
                    break;
                }
                result = message_fetcher.fetch() => {
                    match result {
                        Ok(Some(message)) => {
                            if batch_size > 1 {
                                match Self::fetch_batch(message_fetcher, message, batch_size, batch_timeout).await {
                                    Ok(messages) => {
                                        Self::spawn_batch_task(
                                            messages,
                                            &semaphore,
                                            &self.dispatcher,
                                            &self.retry_policy,
                                            &self.retry_publisher,
                                            &self.dead_letter_publisher,
                                            &self.idempotency_store,
                                            &mut tasks,
                                            self.config.ordered,
                                            self.config.idempotent,
                                            &self.key_locks,
                                        );
                                    }
                                    Err(e) => {
                                        tracing::error!(error = %e, "Failed to fetch message batch");
                                        tokio::time::sleep(error_backoff).await;
                                    }
                                }
                            } else {
                                Self::spawn_message_task(
                                    message,
                                    &semaphore,
                                    &self.dispatcher,
                                    &self.retry_policy,
                                    &self.retry_publisher,
                                    &self.dead_letter_publisher,
                                    &self.idempotency_store,
                                    &mut tasks,
                                    self.config.ordered,
                                    self.config.idempotent,
                                    &self.key_locks,
                                );
                            }
                        }
                        Ok(None) => {
                            tokio::time::sleep(idle_backoff).await;
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Failed to fetch message");
                            tokio::time::sleep(error_backoff).await;
                        }
                    }
                }
            }
        }

        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!(error = %e, "Message handler failed during shutdown"),
                Err(e) => tracing::error!(error = ?e, "Join failed during shutdown"),
            }
        }

        tracing::info!("Consumer runtime stopped gracefully");
        Ok(())
    }

    async fn fetch_batch<MF>(
        message_fetcher: &mut MF,
        first: Message,
        batch_size: usize,
        batch_timeout: Duration,
    ) -> Result<Vec<Message>, ConsumerError>
    where
        MF: MessageFetcher + Send + ?Sized,
    {
        let mut messages = Vec::with_capacity(batch_size);
        messages.push(first);

        if batch_size <= 1 {
            return Ok(messages);
        }

        let timeout = tokio::time::sleep(batch_timeout);
        tokio::pin!(timeout);

        while messages.len() < batch_size {
            tokio::select! {
                _ = &mut timeout => break,
                result = message_fetcher.fetch() => {
                    match result? {
                        Some(message) => messages.push(message),
                        None => break,
                    }
                }
            }
        }

        Ok(messages)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_message_task(
        message: Message,
        semaphore: &Arc<Semaphore>,
        dispatcher: &Arc<dyn Dispatcher>,
        retry_policy: &RetryPolicy,
        retry_publisher: &Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: &Option<Arc<dyn DeadLetterPublisher>>,
        idempotency_store: &Arc<dyn IdempotencyStore>,
        tasks: &mut JoinSet<Result<(), ConsumerError>>,
        ordered: bool,
        idempotent: bool,
        key_locks: &OrderedKeyLocks,
    ) {
        let semaphore = semaphore.clone();
        let dispatcher = dispatcher.clone();
        let key_locks = key_locks.clone();
        let retry_policy = retry_policy.clone();
        let retry_publisher = retry_publisher.clone();
        let dead_letter_publisher = dead_letter_publisher.clone();
        let idempotency_store = idempotent.then(|| idempotency_store.clone());

        tasks.spawn(async move {
            match semaphore.acquire().await {
                Ok(permit) => {
                    let _permit = permit;
                    if ordered {
                        let key = Self::ordered_key(&message);
                        let lock = {
                            let mut locks = key_locks.lock().await;
                            locks
                                .entry(key)
                                .or_insert_with(|| Arc::new(Mutex::new(())))
                                .clone()
                        };
                        let _key_guard = lock.lock().await;
                        Self::process_message(
                            dispatcher,
                            message,
                            retry_policy,
                            retry_publisher,
                            dead_letter_publisher,
                            idempotency_store,
                        )
                        .await
                    } else {
                        Self::process_message(
                            dispatcher,
                            message,
                            retry_policy,
                            retry_publisher,
                            dead_letter_publisher,
                            idempotency_store,
                        )
                        .await
                    }
                }
                Err(_) => {
                    tracing::error!("Consumer semaphore closed");
                    Err(ConsumerError::Configuration(
                        "consumer semaphore closed".into(),
                    ))
                }
            }
        });

        if let Some(result) = tasks.try_join_next() {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!(error = %e, "Message handler failed"),
                Err(e) => tracing::error!(error = ?e, "Join failed"),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_batch_task(
        messages: Vec<Message>,
        semaphore: &Arc<Semaphore>,
        dispatcher: &Arc<dyn Dispatcher>,
        retry_policy: &RetryPolicy,
        retry_publisher: &Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: &Option<Arc<dyn DeadLetterPublisher>>,
        idempotency_store: &Arc<dyn IdempotencyStore>,
        tasks: &mut JoinSet<Result<(), ConsumerError>>,
        ordered: bool,
        idempotent: bool,
        key_locks: &OrderedKeyLocks,
    ) {
        let semaphore = semaphore.clone();
        let dispatcher = dispatcher.clone();
        let key_locks = key_locks.clone();
        let retry_policy = retry_policy.clone();
        let retry_publisher = retry_publisher.clone();
        let dead_letter_publisher = dead_letter_publisher.clone();
        let idempotency_store = idempotent.then(|| idempotency_store.clone());

        tasks.spawn(async move {
            match semaphore.acquire().await {
                Ok(permit) => {
                    let _permit = permit;
                    if ordered {
                        Self::process_ordered_batch(
                            dispatcher,
                            messages,
                            retry_policy,
                            retry_publisher,
                            dead_letter_publisher,
                            idempotency_store,
                            key_locks,
                        )
                        .await
                    } else {
                        Self::process_batch(
                            dispatcher,
                            messages,
                            retry_policy,
                            retry_publisher,
                            dead_letter_publisher,
                            idempotency_store,
                        )
                        .await
                    }
                }
                Err(_) => {
                    tracing::error!("Consumer semaphore closed");
                    Err(ConsumerError::Configuration(
                        "consumer semaphore closed".into(),
                    ))
                }
            }
        });

        if let Some(result) = tasks.try_join_next() {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!(error = %e, "Message batch handler failed"),
                Err(e) => tracing::error!(error = ?e, "Batch join failed"),
            }
        }
    }

    fn ordered_key(message: &Message) -> String {
        message
            .context
            .key
            .clone()
            .unwrap_or_else(|| message.context.topic.clone())
    }

    async fn process_ordered_batch(
        dispatcher: Arc<dyn Dispatcher>,
        messages: Vec<Message>,
        retry_policy: RetryPolicy,
        retry_publisher: Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: Option<Arc<dyn DeadLetterPublisher>>,
        idempotency_store: Option<Arc<dyn IdempotencyStore>>,
        key_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    ) -> Result<(), ConsumerError> {
        if messages.is_empty() {
            return Ok(());
        }

        let mut keys = Vec::new();
        let mut groups: HashMap<String, Vec<Message>> = HashMap::new();
        for message in messages {
            let key = Self::ordered_key(&message);
            if !groups.contains_key(&key) {
                keys.push(key.clone());
            }
            groups.entry(key).or_default().push(message);
        }

        for key in keys {
            let Some(group) = groups.remove(&key) else {
                continue;
            };
            let lock = {
                let mut locks = key_locks.lock().await;
                locks
                    .entry(key)
                    .or_insert_with(|| Arc::new(Mutex::new(())))
                    .clone()
            };
            let _key_guard = lock.lock().await;
            Self::process_batch(
                dispatcher.clone(),
                group,
                retry_policy.clone(),
                retry_publisher.clone(),
                dead_letter_publisher.clone(),
                idempotency_store.clone(),
            )
            .await?;
        }

        Ok(())
    }

    /// 处理单条消息
    async fn process_message(
        dispatcher: Arc<dyn Dispatcher>,
        message: Message,
        retry_policy: RetryPolicy,
        retry_publisher: Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: Option<Arc<dyn DeadLetterPublisher>>,
        idempotency_store: Option<Arc<dyn IdempotencyStore>>,
    ) -> Result<(), ConsumerError> {
        let message_id = message.context.message_id.clone();
        let idempotency_key = Self::idempotency_key(&message);
        let retry_count = message.context.retry_count;
        let failure_message = if retry_publisher.is_some() || dead_letter_publisher.is_some() {
            Some(message.clone())
        } else {
            None
        };

        if Self::is_duplicate(&message, idempotency_store.as_ref()).await? {
            tracing::trace!(
                message_id = %message_id,
                "Duplicate message, skipping"
            );
            if let Some(ack) = message.ack_handle.as_ref() {
                ack.ack().await?;
            }
            return Ok(());
        }

        // 分发消息
        let ack_handle = message.ack_handle.clone();
        let result = match dispatcher.dispatch(message).await {
            Ok(result) => result,
            Err(err) => {
                let failure = FailureContext::from_error("handler_error", &err, retry_count);
                let action = retry_policy.action_for_error(&err, retry_count);
                Self::apply_failure_action(
                    failure_message.as_ref(),
                    ack_handle.as_ref(),
                    &retry_publisher,
                    &dead_letter_publisher,
                    action,
                    failure,
                )
                .await?;
                tracing::warn!(
                    error = %err,
                    message_id = %message_id,
                    action = ?action,
                    "Message handler error handled by failure policy"
                );
                return Ok(());
            }
        };

        // 处理结果
        match result {
            MessageResult::Ack => {
                Self::record_consumed(&idempotency_key, idempotency_store.as_ref()).await?;
                if let Some(ack) = ack_handle.as_ref() {
                    ack.ack().await?;
                }
                tracing::trace!(message_id = %message_id, "Message acknowledged");
            }
            MessageResult::Nack => {
                let action = retry_policy.action_for_nack(retry_count);
                Self::apply_failure_action(
                    failure_message.as_ref(),
                    ack_handle.as_ref(),
                    &retry_publisher,
                    &dead_letter_publisher,
                    action,
                    FailureContext::new("handler_nack", None::<String>, retry_count),
                )
                .await?;
                tracing::warn!(message_id = %message_id, action = ?action, "Message nacked");
            }
            MessageResult::DeadLetter => {
                let action = retry_policy.action_for_dead_letter();
                Self::apply_failure_action(
                    failure_message.as_ref(),
                    ack_handle.as_ref(),
                    &retry_publisher,
                    &dead_letter_publisher,
                    action,
                    FailureContext::new("handler_dead_letter", None::<String>, retry_count),
                )
                .await?;
                tracing::error!(message_id = %message_id, action = ?action, "Message dead-lettered");
            }
        }

        Ok(())
    }

    async fn process_batch(
        dispatcher: Arc<dyn Dispatcher>,
        messages: Vec<Message>,
        retry_policy: RetryPolicy,
        retry_publisher: Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: Option<Arc<dyn DeadLetterPublisher>>,
        idempotency_store: Option<Arc<dyn IdempotencyStore>>,
    ) -> Result<(), ConsumerError> {
        if messages.is_empty() {
            return Ok(());
        }

        let keep_failure_message = retry_publisher.is_some() || dead_letter_publisher.is_some();
        let mut dispatch_messages = Vec::with_capacity(messages.len());
        let mut ack_handles = Vec::with_capacity(messages.len());
        for message in messages {
            if Self::is_duplicate(&message, idempotency_store.as_ref()).await? {
                tracing::trace!(
                    message_id = %message.context.message_id,
                    "Duplicate message in batch, acknowledging without dispatch"
                );
                if let Some(ack) = message.ack_handle.as_ref() {
                    ack.ack().await?;
                }
                continue;
            }

            ack_handles.push((
                message.context.message_id.clone(),
                Self::idempotency_key(&message),
                message.context.retry_count,
                message.ack_handle.clone(),
                keep_failure_message.then(|| message.clone()),
            ));
            dispatch_messages.push(message);
        }

        if dispatch_messages.is_empty() {
            return Ok(());
        }

        let results = match dispatcher.dispatch_batch(dispatch_messages).await {
            Ok(results) => results,
            Err(err) => {
                Self::apply_error_to_all(
                    &ack_handles,
                    &retry_policy,
                    &retry_publisher,
                    &dead_letter_publisher,
                    &err,
                )
                .await?;
                tracing::warn!(error = %err, "Batch handler error handled by failure policy");
                return Ok(());
            }
        };

        if results.len() != ack_handles.len() {
            let error = ConsumerError::Handler(format!(
                "batch handler returned {} results for {} messages",
                results.len(),
                ack_handles.len()
            ));
            Self::apply_error_to_all(
                &ack_handles,
                &retry_policy,
                &retry_publisher,
                &dead_letter_publisher,
                &error,
            )
            .await?;
            return Err(ConsumerError::Handler(format!(
                "batch handler returned {} results for {} messages",
                results.len(),
                ack_handles.len()
            )));
        }

        let mut first_error = None;
        for ((message_id, idempotency_key, retry_count, ack_handle, failure_message), result) in
            ack_handles.iter().zip(results)
        {
            let ack_result = match result {
                MessageResult::Ack => {
                    Self::record_consumed(idempotency_key, idempotency_store.as_ref()).await?;
                    if let Some(ack) = ack_handle.as_ref() {
                        ack.ack().await
                    } else {
                        Ok(())
                    }
                }
                MessageResult::Nack => {
                    tracing::warn!(message_id = %message_id, "Message nacked in batch");
                    let action = retry_policy.action_for_nack(*retry_count);
                    Self::apply_failure_action(
                        failure_message.as_ref(),
                        ack_handle.as_ref(),
                        &retry_publisher,
                        &dead_letter_publisher,
                        action,
                        FailureContext::new("handler_nack", None::<String>, *retry_count),
                    )
                    .await
                }
                MessageResult::DeadLetter => {
                    tracing::error!(message_id = %message_id, "Message sent to DLQ in batch");
                    let action = retry_policy.action_for_dead_letter();
                    Self::apply_failure_action(
                        failure_message.as_ref(),
                        ack_handle.as_ref(),
                        &retry_publisher,
                        &dead_letter_publisher,
                        action,
                        FailureContext::new("handler_dead_letter", None::<String>, *retry_count),
                    )
                    .await
                }
            };

            if let Err(err) = ack_result {
                tracing::warn!(
                    error = %err,
                    message_id = %message_id,
                    "Failed to apply broker ack result for batch message"
                );
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }

        tracing::trace!(batch_size = ack_handles.len(), "Message batch acknowledged");
        Ok(())
    }

    async fn apply_error_to_all(
        ack_handles: &[BatchAckHandle],
        retry_policy: &RetryPolicy,
        retry_publisher: &Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: &Option<Arc<dyn DeadLetterPublisher>>,
        error: &ConsumerError,
    ) -> Result<(), ConsumerError> {
        let mut first_error = None;
        for (message_id, _idempotency_key, retry_count, ack_handle, failure_message) in ack_handles
        {
            let action = retry_policy.action_for_error(error, *retry_count);
            let result = Self::apply_failure_action(
                failure_message.as_ref(),
                ack_handle.as_ref(),
                retry_publisher,
                dead_letter_publisher,
                action,
                FailureContext::from_error("batch_handler_error", error, *retry_count),
            )
            .await;

            if let Err(err) = result {
                tracing::warn!(
                    error = %err,
                    message_id = %message_id,
                    "Failed to apply failure policy after batch error"
                );
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }
        Ok(())
    }

    async fn apply_failure_action(
        message: Option<&Message>,
        ack_handle: Option<&Arc<dyn super::types::MessageAck>>,
        retry_publisher: &Option<Arc<dyn RetryPublisher>>,
        dead_letter_publisher: &Option<Arc<dyn DeadLetterPublisher>>,
        action: FailureAction,
        failure: FailureContext,
    ) -> Result<(), ConsumerError> {
        match action {
            FailureAction::Retry => {
                if let Some(publisher) = retry_publisher {
                    let message = message.ok_or_else(|| {
                        ConsumerError::Retry(
                            "retry publisher requires the original message".to_string(),
                        )
                    })?;
                    publisher.publish_retry(message, failure).await?;
                    if let Some(ack) = ack_handle {
                        ack.ack().await?;
                    }
                    return Ok(());
                }

                if let Some(ack) = ack_handle.as_ref()
                    && ack.supports_native_retry()
                {
                    return ack.nack().await;
                }

                Err(ConsumerError::Retry(
                    "retry publisher is not configured and broker ack handle does not support native retry".to_string(),
                ))
            }
            FailureAction::DeadLetter => {
                let Some(publisher) = dead_letter_publisher else {
                    if let Some(ack) = ack_handle
                        && ack.supports_native_retry()
                    {
                        ack.nack().await?;
                    }
                    return Err(ConsumerError::DeadLetter(
                        "dead-letter publisher is not configured; original message was not termed"
                            .to_string(),
                    ));
                };
                let message = message.ok_or_else(|| {
                    ConsumerError::DeadLetter(
                        "dead-letter publisher requires the original message".to_string(),
                    )
                })?;
                publisher.publish_dead_letter(message, failure).await?;
                if let Some(ack) = ack_handle {
                    ack.term().await?;
                }
                Ok(())
            }
        }
    }

    fn idempotency_key(message: &Message) -> String {
        let message_id = message
            .context
            .headers
            .get("x-envelope-id")
            .filter(|id| !id.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| message.context.message_id.clone());
        if !message_id.trim().is_empty() {
            return format!("{}:{}", message.context.topic, message_id);
        }
        format!(
            "{}:{}:{}",
            message.context.topic, message.context.partition, message.context.offset
        )
    }

    /// 检查消息是否重复（幂等性）
    async fn is_duplicate(
        message: &Message,
        store: Option<&Arc<dyn IdempotencyStore>>,
    ) -> Result<bool, ConsumerError> {
        let Some(store) = store else {
            return Ok(false);
        };
        store.is_consumed(&Self::idempotency_key(message)).await
    }

    /// 记录消息已消费
    async fn record_consumed(
        message_id: &str,
        store: Option<&Arc<dyn IdempotencyStore>>,
    ) -> Result<(), ConsumerError> {
        let Some(store) = store else {
            return Ok(());
        };
        store.record_consumed(message_id).await
    }
}

/// 将 [ConsumerRuntime] + [MessageFetcher] 适配为 [MqConsumer]，供 [crate::runtime::ServiceRuntime] 统一调度
pub struct ConsumerRuntimeTask {
    runtime: Arc<ConsumerRuntime>,
    fetcher: Arc<tokio::sync::Mutex<Box<dyn MessageFetcher + Send>>>,
}

impl ConsumerRuntimeTask {
    pub fn new(runtime: ConsumerRuntime, fetcher: impl MessageFetcher + 'static) -> Self {
        Self {
            runtime: Arc::new(runtime),
            fetcher: Arc::new(tokio::sync::Mutex::new(Box::new(fetcher))),
        }
    }

    /// 与 [ConsumerRuntime::new] 参数一致，便于链式构建
    pub fn from_parts(
        config: ConsumerConfig,
        dispatcher: Arc<dyn Dispatcher>,
        fetcher: impl MessageFetcher + 'static,
    ) -> Self {
        Self::new(ConsumerRuntime::new(config, dispatcher), fetcher)
    }

    pub fn from_parts_with_failure_publishers(
        config: ConsumerConfig,
        dispatcher: Arc<dyn Dispatcher>,
        fetcher: impl MessageFetcher + 'static,
        failure_publishers: ConsumerFailurePublishers,
    ) -> Self {
        Self::new(
            ConsumerRuntime::new(config, dispatcher).with_failure_publishers(failure_publishers),
            fetcher,
        )
    }
}

impl MqConsumer for ConsumerRuntimeTask {
    fn consume(
        &self,
        shutdown_rx: oneshot::Receiver<()>,
    ) -> Pin<Box<dyn Future<Output = TaskResult> + Send + '_>> {
        let runtime = self.runtime.clone();
        let fetcher = self.fetcher.clone();
        Box::pin(async move {
            let mut fetcher = fetcher.lock().await;
            runtime
                .run_with_shutdown(&mut *fetcher, shutdown_rx)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
        })
    }
}

/// 消息获取器 Trait
#[async_trait::async_trait]
pub trait MessageFetcher: Send {
    /// 获取下一条消息
    async fn fetch(&mut self) -> Result<Option<Message>, ConsumerError>;
}

#[async_trait::async_trait]
impl MessageFetcher for Box<dyn MessageFetcher + Send> {
    async fn fetch(&mut self) -> Result<Option<Message>, ConsumerError> {
        self.as_mut().fetch().await
    }
}

/// 消息消费统计
#[derive(Debug, Default, Clone)]
pub struct ConsumerStats {
    pub total_messages: u64,
    pub processed_messages: u64,
    pub failed_messages: u64,
    pub retried_messages: u64,
    pub dlq_messages: u64,
}

impl ConsumerStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn increment_processed(&mut self) {
        self.processed_messages += 1;
        self.total_messages += 1;
    }

    pub fn increment_failed(&mut self) {
        self.failed_messages += 1;
    }

    pub fn increment_retried(&mut self) {
        self.retried_messages += 1;
    }

    pub fn increment_dlq(&mut self) {
        self.dlq_messages += 1;
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_messages == 0 {
            return 1.0;
        }
        self.processed_messages as f64 / self.total_messages as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mq::consumer::types::{ContentType, MessageAck, MessageContext};
    use flare_core_base::context::Context;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingAck {
        acked: Arc<AtomicUsize>,
        nacked: Arc<AtomicUsize>,
        termed: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl MessageAck for CountingAck {
        async fn ack(&self) -> Result<(), ConsumerError> {
            self.acked.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn nack(&self) -> Result<(), ConsumerError> {
            self.nacked.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn term(&self) -> Result<(), ConsumerError> {
            self.termed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct BatchOnlyDispatcher {
        batch_calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl Dispatcher for BatchOnlyDispatcher {
        async fn dispatch(&self, _message: Message) -> Result<MessageResult, ConsumerError> {
            Err(ConsumerError::Handler(
                "single dispatch should not be used".to_string(),
            ))
        }

        async fn dispatch_batch(
            &self,
            messages: Vec<Message>,
        ) -> Result<Vec<MessageResult>, ConsumerError> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(ConsumerError::Handler("batch failed".to_string()));
            }
            Ok(vec![MessageResult::Ack; messages.len()])
        }

        fn register(
            &mut self,
            _topic: String,
            _handler: Arc<dyn super::super::handler::MessageHandler>,
        ) -> Result<(), ConsumerError> {
            Ok(())
        }

        fn topics(&self) -> Vec<String> {
            vec!["test.topic".to_string()]
        }
    }

    struct RecordingBatchDispatcher {
        batches: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[async_trait::async_trait]
    impl Dispatcher for RecordingBatchDispatcher {
        async fn dispatch(&self, _message: Message) -> Result<MessageResult, ConsumerError> {
            Err(ConsumerError::Handler(
                "single dispatch should not be used".to_string(),
            ))
        }

        async fn dispatch_batch(
            &self,
            messages: Vec<Message>,
        ) -> Result<Vec<MessageResult>, ConsumerError> {
            let ids = messages
                .iter()
                .map(|message| message.context.message_id.clone())
                .collect::<Vec<_>>();
            self.batches.lock().await.push(ids);
            Ok(vec![MessageResult::Ack; messages.len()])
        }

        fn register(
            &mut self,
            _topic: String,
            _handler: Arc<dyn super::super::handler::MessageHandler>,
        ) -> Result<(), ConsumerError> {
            Ok(())
        }

        fn topics(&self) -> Vec<String> {
            vec!["test.topic".to_string()]
        }
    }

    struct ResultDispatcher {
        result: MessageResult,
    }

    #[async_trait::async_trait]
    impl Dispatcher for ResultDispatcher {
        async fn dispatch(&self, _message: Message) -> Result<MessageResult, ConsumerError> {
            Ok(self.result.clone())
        }

        async fn dispatch_batch(
            &self,
            messages: Vec<Message>,
        ) -> Result<Vec<MessageResult>, ConsumerError> {
            Ok(vec![self.result.clone(); messages.len()])
        }

        fn register(
            &mut self,
            _topic: String,
            _handler: Arc<dyn super::super::handler::MessageHandler>,
        ) -> Result<(), ConsumerError> {
            Ok(())
        }

        fn topics(&self) -> Vec<String> {
            vec!["test.topic".to_string()]
        }
    }

    struct CountingResultDispatcher {
        result: MessageResult,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Dispatcher for CountingResultDispatcher {
        async fn dispatch(&self, _message: Message) -> Result<MessageResult, ConsumerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.result.clone())
        }

        async fn dispatch_batch(
            &self,
            messages: Vec<Message>,
        ) -> Result<Vec<MessageResult>, ConsumerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![self.result.clone(); messages.len()])
        }

        fn register(
            &mut self,
            _topic: String,
            _handler: Arc<dyn super::super::handler::MessageHandler>,
        ) -> Result<(), ConsumerError> {
            Ok(())
        }

        fn topics(&self) -> Vec<String> {
            vec!["test.topic".to_string()]
        }
    }

    struct CountingRetryPublisher {
        published: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl RetryPublisher for CountingRetryPublisher {
        async fn publish_retry(
            &self,
            _message: &Message,
            _failure: FailureContext,
        ) -> Result<(), ConsumerError> {
            self.published.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct CountingDeadLetterPublisher {
        published: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl DeadLetterPublisher for CountingDeadLetterPublisher {
        async fn publish_dead_letter(
            &self,
            _message: &Message,
            _failure: FailureContext,
        ) -> Result<(), ConsumerError> {
            self.published.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn test_message(id: &str, ack_handle: Arc<dyn MessageAck>) -> Message {
        let ctx = Arc::new(Context::with_request_id(format!("req-{id}")));
        let mut context = MessageContext::new(ctx, "test.topic".to_string());
        context.message_id = id.to_string();
        Message::new(Vec::new(), ContentType::Raw, context).with_ack_handle(ack_handle)
    }

    fn keyed_test_message(id: &str, key: &str, ack_handle: Arc<dyn MessageAck>) -> Message {
        let mut message = test_message(id, ack_handle);
        message.context.key = Some(key.to_string());
        message
    }

    fn default_retry_policy() -> RetryPolicy {
        RetryPolicy::new(3, true)
    }

    #[tokio::test]
    async fn process_batch_acks_all_messages_after_batch_success() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let batch_calls = Arc::new(AtomicUsize::new(0));
        let dispatcher = Arc::new(BatchOnlyDispatcher {
            batch_calls: batch_calls.clone(),
            fail: false,
        });
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });

        let messages = vec![
            test_message("a", ack_handle.clone()),
            test_message("b", ack_handle.clone()),
            test_message("c", ack_handle),
        ];

        ConsumerRuntime::process_batch(
            dispatcher,
            messages,
            default_retry_policy(),
            None,
            None,
            None,
        )
        .await
        .expect("batch should succeed");

        assert_eq!(batch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(acked.load(Ordering::SeqCst), 3);
        assert_eq!(nacked.load(Ordering::SeqCst), 0);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_batch_nacks_all_messages_after_batch_failure() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let batch_calls = Arc::new(AtomicUsize::new(0));
        let dispatcher = Arc::new(BatchOnlyDispatcher {
            batch_calls: batch_calls.clone(),
            fail: true,
        });
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });

        let messages = vec![
            test_message("a", ack_handle.clone()),
            test_message("b", ack_handle.clone()),
            test_message("c", ack_handle),
        ];

        ConsumerRuntime::process_batch(
            dispatcher,
            messages,
            default_retry_policy(),
            None,
            None,
            None,
        )
        .await
        .expect("batch failure should be handled by native retry");

        assert_eq!(batch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(acked.load(Ordering::SeqCst), 0);
        assert_eq!(nacked.load(Ordering::SeqCst), 3);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_ordered_batch_groups_by_key_and_preserves_key_order() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let batches = Arc::new(Mutex::new(Vec::new()));
        let dispatcher = Arc::new(RecordingBatchDispatcher {
            batches: batches.clone(),
        });
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });

        let messages = vec![
            keyed_test_message("a1", "user-a", ack_handle.clone()),
            keyed_test_message("b1", "user-b", ack_handle.clone()),
            keyed_test_message("a2", "user-a", ack_handle.clone()),
            keyed_test_message("b2", "user-b", ack_handle),
        ];

        ConsumerRuntime::process_ordered_batch(
            dispatcher,
            messages,
            default_retry_policy(),
            None,
            None,
            None,
            Arc::new(Mutex::new(HashMap::new())),
        )
        .await
        .expect("ordered batch should succeed");

        assert_eq!(
            *batches.lock().await,
            vec![
                vec!["a1".to_string(), "a2".to_string()],
                vec!["b1".to_string(), "b2".to_string()],
            ]
        );
        assert_eq!(acked.load(Ordering::SeqCst), 4);
        assert_eq!(nacked.load(Ordering::SeqCst), 0);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_message_acks_duplicate_without_dispatch() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });
        let message = test_message("duplicate", ack_handle);
        let key = ConsumerRuntime::idempotency_key(&message);
        let store = Arc::new(InMemoryIdempotencyStore::default());
        store.record_consumed(&key).await.unwrap();

        ConsumerRuntime::process_message(
            Arc::new(CountingResultDispatcher {
                result: MessageResult::Ack,
                calls: calls.clone(),
            }),
            message,
            default_retry_policy(),
            None,
            None,
            Some(store),
        )
        .await
        .expect("duplicate should be acknowledged without dispatch");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(acked.load(Ordering::SeqCst), 1);
        assert_eq!(nacked.load(Ordering::SeqCst), 0);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_message_records_consumed_after_ack() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });
        let message = test_message("fresh", ack_handle);
        let key = ConsumerRuntime::idempotency_key(&message);
        let store = Arc::new(InMemoryIdempotencyStore::default());

        ConsumerRuntime::process_message(
            Arc::new(CountingResultDispatcher {
                result: MessageResult::Ack,
                calls: calls.clone(),
            }),
            message,
            default_retry_policy(),
            None,
            None,
            Some(store.clone()),
        )
        .await
        .expect("fresh message should dispatch and record idempotency");

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(acked.load(Ordering::SeqCst), 1);
        assert!(store.is_consumed(&key).await.unwrap());
        assert_eq!(nacked.load(Ordering::SeqCst), 0);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_message_publishes_retry_then_acks_original() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let retry_published = Arc::new(AtomicUsize::new(0));
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });
        let retry_publisher: Arc<dyn RetryPublisher> = Arc::new(CountingRetryPublisher {
            published: retry_published.clone(),
        });

        ConsumerRuntime::process_message(
            Arc::new(ResultDispatcher {
                result: MessageResult::Nack,
            }),
            test_message("retry", ack_handle),
            default_retry_policy(),
            Some(retry_publisher),
            None,
            None,
        )
        .await
        .expect("retry handoff should succeed");

        assert_eq!(retry_published.load(Ordering::SeqCst), 1);
        assert_eq!(acked.load(Ordering::SeqCst), 1);
        assert_eq!(nacked.load(Ordering::SeqCst), 0);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_message_publishes_dlq_then_terms_original() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let dlq_published = Arc::new(AtomicUsize::new(0));
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });
        let dlq_publisher: Arc<dyn DeadLetterPublisher> = Arc::new(CountingDeadLetterPublisher {
            published: dlq_published.clone(),
        });

        ConsumerRuntime::process_message(
            Arc::new(ResultDispatcher {
                result: MessageResult::DeadLetter,
            }),
            test_message("dlq", ack_handle),
            default_retry_policy(),
            None,
            Some(dlq_publisher),
            None,
        )
        .await
        .expect("DLQ handoff should succeed");

        assert_eq!(dlq_published.load(Ordering::SeqCst), 1);
        assert_eq!(acked.load(Ordering::SeqCst), 0);
        assert_eq!(nacked.load(Ordering::SeqCst), 0);
        assert_eq!(termed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn process_message_without_dlq_publisher_does_not_term_original() {
        let acked = Arc::new(AtomicUsize::new(0));
        let nacked = Arc::new(AtomicUsize::new(0));
        let termed = Arc::new(AtomicUsize::new(0));
        let ack_handle: Arc<dyn MessageAck> = Arc::new(CountingAck {
            acked: acked.clone(),
            nacked: nacked.clone(),
            termed: termed.clone(),
        });

        ConsumerRuntime::process_message(
            Arc::new(ResultDispatcher {
                result: MessageResult::DeadLetter,
            }),
            test_message("dlq-missing", ack_handle),
            default_retry_policy(),
            None,
            None,
            None,
        )
        .await
        .expect_err("missing DLQ publisher must not silently term the message");

        assert_eq!(acked.load(Ordering::SeqCst), 0);
        assert_eq!(nacked.load(Ordering::SeqCst), 1);
        assert_eq!(termed.load(Ordering::SeqCst), 0);
    }
}
