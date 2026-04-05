//! MQ 消费者运行时
//!
//! 负责消费循环、并发控制、优雅关闭等运行时功能。
//! 轮询/退避/并发与 [crate::runtime::config::PollWorkerConfig]、[crate::runtime::config::RuntimeConfig] 对齐。
//!
//! 与 [super::task::MqConsumer] / [crate::runtime::ServiceRuntime] 集成：使用 [ConsumerRuntimeTask]，
//! 通过 [crate::runtime::ServiceRuntime::add_mq_consumer] 或 [crate::runtime::ServiceRuntime::add_mq_consumer_runtime] 注册。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use flare_core_runtime::config::{PollWorkerConfig, RuntimeConfig};
use flare_core_runtime::task::TaskResult;

use super::task::MqConsumer;

use super::dispatcher::Dispatcher;
use super::types::{ConsumerError, Message, MessageResult};

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
    /// Kafka：覆盖 `KafkaConsumerConfig::consumer_group`（`None` 时使用传入的 Kafka 配置）
    pub kafka_consumer_group_override: Option<String>,
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
            kafka_consumer_group_override: None,
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

    pub fn with_ordered(mut self, ordered: bool) -> Self {
        self.ordered = ordered;
        self
    }

    pub fn with_idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = idempotent;
        self
    }

    /// Kafka：设置 `group.id` 覆盖（与 [crate::mq::kafka::KafkaMessageFetcher::new_with_consumer_group] 一致）
    pub fn with_kafka_consumer_group(mut self, group_id: impl Into<String>) -> Self {
        self.kafka_consumer_group_override = Some(group_id.into());
        self
    }
}

/// 消费者运行时
pub struct ConsumerRuntime {
    config: ConsumerConfig,
    dispatcher: Arc<dyn Dispatcher>,
}

impl ConsumerRuntime {
    /// 创建新的消费者运行时
    pub fn new(config: ConsumerConfig, dispatcher: Arc<dyn Dispatcher>) -> Self {
        Self { config, dispatcher }
    }

    /// 持续拉取并处理消息，直到进程退出或 `fetch` 侧永久失败（与 [Self::run_with_shutdown] 相对，用于独立 Kafka/NATS 入口）
    pub async fn run<MF>(&self, message_fetcher: &mut MF) -> Result<(), ConsumerError>
    where
        MF: MessageFetcher + Send + ?Sized,
    {
        let semaphore = Arc::new(Semaphore::new(self.config.poll.concurrency));
        let mut tasks: JoinSet<Result<(), ConsumerError>> = JoinSet::new();
        let idle_backoff = self.config.poll.idle_backoff;
        let error_backoff = self.config.poll.error_backoff;

        loop {
            match message_fetcher.fetch().await {
                Ok(Some(message)) => {
                    Self::spawn_message_task(message, &semaphore, &self.dispatcher, &mut tasks);
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

        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    tracing::info!("Shutdown signal received, stopping consumer");
                    break;
                }
                result = message_fetcher.fetch() => {
                    match result {
                        Ok(Some(message)) => {
                            Self::spawn_message_task(
                                message,
                                &semaphore,
                                &self.dispatcher,
                                &mut tasks,
                            );
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

    fn spawn_message_task(
        message: Message,
        semaphore: &Arc<Semaphore>,
        dispatcher: &Arc<dyn Dispatcher>,
        tasks: &mut JoinSet<Result<(), ConsumerError>>,
    ) {
        let semaphore = semaphore.clone();
        let dispatcher = dispatcher.clone();

        tasks.spawn(async move {
            match semaphore.acquire().await {
                Ok(permit) => {
                    let _permit = permit;
                    Self::process_message(dispatcher, message).await
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

    /// 处理单条消息
    async fn process_message(
        dispatcher: Arc<dyn Dispatcher>,
        message: Message,
    ) -> Result<(), ConsumerError> {
        let ctx = &message.context;
        let message_id = ctx.message_id.clone();

        // 幂等性检查
        if Self::is_duplicate(&message) {
            tracing::debug!(
                message_id = %message_id,
                "Duplicate message, skipping"
            );
            return Ok(());
        }

        // 记录消费
        Self::record_consumed(&message);

        // 分发消息
        let result = dispatcher.dispatch(message).await?;

        // 处理结果
        match result {
            MessageResult::Ack => {
                tracing::debug!(message_id = %message_id, "Message acknowledged");
            }
            MessageResult::Nack => {
                tracing::warn!(message_id = %message_id, "Message nacked");
            }
            MessageResult::DeadLetter => {
                tracing::error!(message_id = %message_id, "Message sent to DLQ");
                // TODO: 发送到 DLQ
            }
        }

        Ok(())
    }

    /// 检查消息是否重复（幂等性）
    fn is_duplicate(_message: &Message) -> bool {
        // TODO: 实现幂等性检查（基于 Redis 或数据库）
        false
    }

    /// 记录消息已消费
    fn record_consumed(_message: &Message) {
        // TODO: 记录消费状态（用于幂等性检查）
    }
}

/// 将 [ConsumerRuntime] + [MessageFetcher] 适配为 [MqConsumer]，供 [crate::runtime::ServiceRuntime] 统一调度
pub struct ConsumerRuntimeTask {
    runtime: Arc<ConsumerRuntime>,
    fetcher: Arc<tokio::sync::Mutex<Box<dyn MessageFetcher + Send>>>,
}

impl ConsumerRuntimeTask {
    pub fn new(runtime: ConsumerRuntime, fetcher: impl MessageFetcher + Send + 'static) -> Self {
        Self {
            runtime: Arc::new(runtime),
            fetcher: Arc::new(tokio::sync::Mutex::new(Box::new(fetcher))),
        }
    }

    /// 与 [ConsumerRuntime::new] 参数一致，便于链式构建
    pub fn from_parts(
        config: ConsumerConfig,
        dispatcher: Arc<dyn Dispatcher>,
        fetcher: impl MessageFetcher + Send + 'static,
    ) -> Self {
        Self::new(ConsumerRuntime::new(config, dispatcher), fetcher)
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
