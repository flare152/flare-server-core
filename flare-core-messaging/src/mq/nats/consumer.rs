//! NATS JetStream 消费者实现
//!
//! 实现基于 NATS JetStream 的消息消费，支持并发消费、Context 透传等特性。

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use async_nats::jetstream;
use async_nats::jetstream::Message as NatsMessage;
use async_nats::jetstream::consumer::pull::Config as PullConfig;
use async_nats::jetstream::consumer::pull::Stream as PullStream;
use futures_util::StreamExt;

use super::super::process_ack_metrics::record_process_ack;
use super::config::{
    NatsConsumerConfig, NatsProducerConfig, NatsStreamSpec, resolve_subject_stream,
};
use crate::mq::consumer::dispatcher::Dispatcher;
use crate::mq::consumer::failure::{ConsumerFailurePublishers, retry_count_from_headers};
use crate::mq::consumer::{
    ConsumerConfig, ConsumerError, ConsumerRuntimeTask, ContentType, Message, MessageAck,
    MessageContext, MessageFetcher, MqConsumerTask,
};

pub fn context_from_nats_headers(
    headers: &std::collections::HashMap<String, String>,
) -> flare_core_base::context::Ctx {
    crate::mq::context::mq_headers_to_ctx(headers)
}

/// NATS JetStream 消息获取器
pub struct NatsMessageFetcher {
    stream: PullStream,
    manual_ack: bool,
}

#[derive(Debug, Clone)]
struct ResolvedNatsConsumerConfig {
    nats_url: String,
    consumer_group: String,
    manual_ack: bool,
    batch_size: usize,
    batch_timeout_ms: u64,
    durable: bool,
    stream: NatsStreamSpec,
}

impl ResolvedNatsConsumerConfig {
    fn from_config<C>(config: &C, stream: NatsStreamSpec) -> Self
    where
        C: NatsConsumerConfig + Send + Sync,
    {
        Self {
            nats_url: config.nats_url().to_string(),
            consumer_group: config.consumer_group().to_string(),
            manual_ack: config.enable_manual_ack(),
            batch_size: config.batch_size(),
            batch_timeout_ms: config.batch_timeout_ms(),
            durable: config.enable_durable(),
            stream,
        }
    }
}

impl NatsConsumerConfig for ResolvedNatsConsumerConfig {
    fn nats_url(&self) -> &str {
        &self.nats_url
    }

    fn consumer_group(&self) -> &str {
        &self.consumer_group
    }

    fn enable_manual_ack(&self) -> bool {
        self.manual_ack
    }

    fn batch_size(&self) -> usize {
        self.batch_size
    }

    fn batch_timeout_ms(&self) -> u64 {
        self.batch_timeout_ms
    }

    fn enable_durable(&self) -> bool {
        self.durable
    }

    fn stream_specs(&self) -> Vec<NatsStreamSpec> {
        vec![self.stream.clone()]
    }
}

impl NatsMessageFetcher {
    /// 创建新的 NATS JetStream 消息获取器
    pub async fn new<C>(config: &C, subjects: Vec<String>) -> Result<Self, ConsumerError>
    where
        C: NatsConsumerConfig + Send + Sync,
    {
        let client = async_nats::connect(&config.nats_url())
            .await
            .map_err(|e| ConsumerError::Connection(e.to_string()))?;

        let context = jetstream::new(client);
        let manual_ack = config.enable_manual_ack();

        // 创建或更新业务 stream。业务只传 subjects，JetStream stream 由 core 统一解析。
        let stream_spec = resolve_stream_for_subjects(config, &subjects)?;
        let stream_name = stream_spec.name.clone();
        let stream = jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: stream_spec.subjects.clone(),
            retention: jetstream::stream::RetentionPolicy::Limits,
            max_age: stream_spec.max_age,
            max_bytes: stream_spec.max_bytes,
            // 满配额时丢最旧的（已消费的历史），而不是拒绝新 publish。
            // 默认的 DiscardPolicy::New 会把「流写满」升级成「全站发不出消息」。
            discard: jetstream::stream::DiscardPolicy::Old,
            storage: jetstream::stream::StorageType::File,
            num_replicas: stream_spec.num_replicas,
            duplicate_window: stream_spec.duplicate_window,
            ..Default::default()
        };

        context.create_or_update_stream(stream).await.map_err(|e| {
            let raw = e.to_string();
            ConsumerError::Configuration(
                super::config::explain_jetstream_resource_error(
                    &format!(
                        "创建/更新 stream {stream_name}（本条申请 {} MiB）",
                        stream_spec.max_bytes / (1024 * 1024)
                    ),
                    &raw,
                )
                .unwrap_or(raw),
            )
        })?;

        // 创建 consumer
        let consumer_name = if config.enable_durable() {
            durable_consumer_name(config.consumer_group(), &stream_name, &subjects)
        } else {
            "ephemeral_consumer".to_string()
        };

        let consumer_name_for_prune = consumer_name.clone();
        let consumer_config = PullConfig {
            name: Some(consumer_name.clone()),
            durable_name: if config.enable_durable() {
                Some(consumer_name)
            } else {
                None
            },
            ack_policy: if manual_ack {
                jetstream::consumer::AckPolicy::Explicit
            } else {
                jetstream::consumer::AckPolicy::None
            },
            filter_subjects: subjects.clone(),
            // 只对**新建**的 durable 生效：已存在的 durable 由服务端保留原有游标。
            deliver_policy: if config.deliver_from_new() {
                jetstream::consumer::DeliverPolicy::New
            } else {
                jetstream::consumer::DeliverPolicy::All
            },
            ack_wait: std::time::Duration::from_secs(config.ack_wait_secs().max(1)),
            max_deliver: config.max_deliver().max(1),
            max_ack_pending: config.max_ack_pending().max(1),
            max_batch: config.batch_size().max(1) as i64,
            max_expires: std::time::Duration::from_millis(config.batch_timeout_ms().max(1)),
            ..Default::default()
        };

        let consumer = context
            .create_consumer_on_stream(consumer_config, stream_name.clone())
            .await
            .map_err(|e| ConsumerError::Configuration(e.to_string()))?;

        // durable 名字里含订阅 subject 列表的哈希：给消费组**加一条 subject**
        // 就会生成全新的 durable，而旧的那个仍留在服务端、再也没人消费。
        // JetStream 会一直为它保留消息，积压只增不减——流因此永远无法老化，
        // 最终撑爆存储配额（本仓出过一次 publish 全 503 的故障）。
        // 线上实测就留下过一个卡在 seq 979841、积压 11.9 万条的孤儿。
        //
        // 每次订阅成功后顺手清掉**同组同流但名字不同**的陈旧 durable。
        // 清理是尽力而为：失败只告警，不该拦住服务起来。
        if config.enable_durable() {
            Self::prune_stale_durables(
                &context,
                &stream_name,
                config.consumer_group(),
                &consumer_name_for_prune,
            )
            .await;
        }

        // Keep the client pull request within the server-side limits declared above.
        // async-nats::Consumer::messages() defaults to batch=200/expires=30s, which
        // can be rejected when the durable is capped by max_batch/max_expires.
        let stream = consumer
            .stream()
            .max_messages_per_batch(config.batch_size().max(1))
            .expires(std::time::Duration::from_millis(
                config.batch_timeout_ms().max(1),
            ))
            .messages()
            .await
            .map_err(|e| ConsumerError::Configuration(e.to_string()))?;

        tracing::info!(
            stream = %stream_name,
            consumer_group = %config.consumer_group(),
            subjects = ?subjects,
            batch_size = config.batch_size().max(1),
            batch_timeout_ms = config.batch_timeout_ms().max(1),
            "Subscribed to NATS subjects"
        );
        let fetcher = Self { stream, manual_ack };

        Ok(fetcher)
    }

    /// 清掉同组同流、但名字与当前不同的陈旧 durable。
    ///
    /// 名字形如 `consumer_<组>_<流>_<subject哈希>`，"同组同流不同哈希"
    /// 就是改过 subject 之后遗留的旧 durable：不会再被消费，
    /// 却让 JetStream 一直保留消息、积压只增不减。
    ///
    /// 只按前缀 `consumer_<组>_<流>_` 匹配，不会误伤别的消费组。
    async fn prune_stale_durables(
        context: &jetstream::Context,
        stream_name: &str,
        group: &str,
        keep: &str,
    ) {
        use futures::StreamExt as _;

        let prefix = format!(
            "consumer_{}_{}_",
            sanitize_task_part(group),
            sanitize_task_part(stream_name)
        );
        let Ok(stream) = context.get_stream(stream_name).await else {
            tracing::warn!(stream = %stream_name, "清理陈旧 durable：取不到流，跳过");
            return;
        };
        let mut names = stream.consumer_names();
        let mut stale = Vec::new();
        while let Some(item) = names.next().await {
            match item {
                Ok(name) if name.starts_with(&prefix) && name != keep => stale.push(name),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(stream = %stream_name, error = %e, "清理陈旧 durable：列举失败，跳过");
                    return;
                }
            }
        }
        for name in stale {
            match stream.delete_consumer(&name).await {
                Ok(_) => tracing::info!(
                    stream = %stream_name,
                    consumer_group = %group,
                    removed = %name,
                    kept = %keep,
                    "清掉改 subject 后遗留的陈旧 durable"
                ),
                Err(e) => tracing::warn!(
                    stream = %stream_name, consumer = %name, error = %e,
                    "清理陈旧 durable：删除失败（不影响启动）"
                ),
            }
        }
    }
}

/// 根据 dispatcher 注册的 subjects 自动拆分为多个 JetStream consumer task。
///
/// Kafka 的一个 consumer 可以同时订阅多个 topic；JetStream 的 durable consumer 必须绑定单个
/// stream。本 builder 将这种差异收敛在 core 内部，业务侧仍只注册 subject handler。
pub async fn build_nats_consumer_tasks<C>(
    config: &C,
    consumer_config: ConsumerConfig,
    dispatcher: Arc<dyn Dispatcher>,
    task_name_prefix: impl AsRef<str>,
) -> Result<Vec<MqConsumerTask>, ConsumerError>
where
    C: NatsConsumerConfig + Send + Sync,
{
    build_nats_consumer_tasks_with_failure_publishers(
        config,
        consumer_config,
        dispatcher,
        task_name_prefix,
        ConsumerFailurePublishers::default(),
    )
    .await
}

/// 死信生产者配置包装:连接参数沿用服务配置,但 `stream_specs` 只含死信流,确保 `FLARE_DLQ` stream 存在
/// (不依赖服务自身拓扑是否包含死信流)。
struct DlqProducerConfig<'a, C: NatsProducerConfig> {
    inner: &'a C,
}

impl<C: NatsProducerConfig> NatsProducerConfig for DlqProducerConfig<'_, C> {
    fn nats_url(&self) -> &str {
        self.inner.nats_url()
    }
    fn timeout_ms(&self) -> u64 {
        self.inner.timeout_ms()
    }
    fn retries(&self) -> u32 {
        self.inner.retries()
    }
    fn retry_backoff_ms(&self) -> u64 {
        self.inner.retry_backoff_ms()
    }
    fn stream_specs(&self) -> Vec<NatsStreamSpec> {
        vec![super::config::dlq_stream_spec()]
    }
}

/// 与 [`build_nats_consumer_tasks`] 相同,但额外接通**死信(DLQ)发布器**:重试耗尽/不可重试的消息不再
/// 在 stream 里无限重投(毒消息循环),而是投到独立死信流的 `dlq_subject`(`flare.im.dlq.<service>`,见
/// [`super::config::default_stream_specs`]),长留存以便排查/重放。`dlq_subject` 必须**落在死信流通配
/// `flare.im.dlq.>` 之内、且不与本消费者订阅的 subject 重叠**,否则死信会被重新消费形成循环。
pub async fn build_nats_consumer_tasks_with_dlq<C>(
    config: &C,
    consumer_config: ConsumerConfig,
    dispatcher: Arc<dyn Dispatcher>,
    task_name_prefix: impl AsRef<str>,
    dlq_subject: impl Into<String>,
) -> Result<Vec<MqConsumerTask>, ConsumerError>
where
    C: NatsConsumerConfig + NatsProducerConfig + Send + Sync,
{
    // 死信生产者**只确保死信流**(FLARE_DLQ),与服务自身配置的拓扑无关——否则若部署用自定义拓扑(非
    // default_stream_specs),死信流可能缺失导致死信投递无 stream 而失败。
    let dlq_producer_config = DlqProducerConfig { inner: config };
    let producer = super::producer::NatsProducer::new(&dlq_producer_config)
        .await
        .map_err(|e| ConsumerError::Configuration(format!("build DLQ producer: {e}")))?;
    let dead_letter: Arc<dyn crate::mq::consumer::DeadLetterPublisher> =
        Arc::new(crate::mq::consumer::ProducerDeadLetterPublisher::new(
            Arc::new(producer),
            crate::mq::consumer::FailureTopic::fixed(dlq_subject),
        ));
    let failure_publishers = ConsumerFailurePublishers {
        retry: None,
        dead_letter: Some(dead_letter),
    };
    build_nats_consumer_tasks_with_failure_publishers(
        config,
        consumer_config,
        dispatcher,
        task_name_prefix,
        failure_publishers,
    )
    .await
}

pub async fn build_nats_consumer_tasks_with_failure_publishers<C>(
    config: &C,
    consumer_config: ConsumerConfig,
    dispatcher: Arc<dyn Dispatcher>,
    task_name_prefix: impl AsRef<str>,
    failure_publishers: ConsumerFailurePublishers,
) -> Result<Vec<MqConsumerTask>, ConsumerError>
where
    C: NatsConsumerConfig + Send + Sync,
{
    let mut subjects = dispatcher.topics();
    if subjects.is_empty() {
        return Err(ConsumerError::Configuration(
            "NATS consumer dispatcher has no registered subjects".to_string(),
        ));
    }

    subjects.sort();
    subjects.dedup();

    let grouped = group_subjects_by_stream(config, &subjects)?;
    let mut tasks = Vec::with_capacity(grouped.len());
    let task_name_prefix = task_name_prefix.as_ref();

    for (stream_name, (stream, stream_subjects)) in grouped {
        let resolved_config = ResolvedNatsConsumerConfig::from_config(config, stream);
        let fetcher = NatsMessageFetcher::new(&resolved_config, stream_subjects.clone()).await?;
        let consumer = ConsumerRuntimeTask::from_parts_with_failure_publishers(
            consumer_config.clone(),
            dispatcher.clone(),
            fetcher,
            failure_publishers.clone(),
        );
        let task_name = format!("{}-{}", task_name_prefix, sanitize_task_part(&stream_name));
        tasks.push(MqConsumerTask::new(task_name, Box::new(consumer)));
    }

    Ok(tasks)
}

fn group_subjects_by_stream<C>(
    config: &C,
    subjects: &[String],
) -> Result<BTreeMap<String, (NatsStreamSpec, Vec<String>)>, ConsumerError>
where
    C: NatsConsumerConfig + Send + Sync,
{
    let specs = config.stream_specs();
    let mut grouped: BTreeMap<String, (NatsStreamSpec, Vec<String>)> = BTreeMap::new();

    for subject in subjects {
        let stream = resolve_subject_stream(&specs, subject)
            .cloned()
            .ok_or_else(|| {
                ConsumerError::Configuration(format!(
                    "No JetStream stream configured for subject: {}",
                    subject
                ))
            })?;

        grouped
            .entry(stream.name.clone())
            .or_insert_with(|| (stream, Vec::new()))
            .1
            .push(subject.clone());
    }

    Ok(grouped)
}

fn resolve_stream_for_subjects<C>(
    config: &C,
    subjects: &[String],
) -> Result<NatsStreamSpec, ConsumerError>
where
    C: NatsConsumerConfig + Send + Sync,
{
    let grouped = group_subjects_by_stream(config, subjects)?;
    if grouped.len() == 1 {
        let mut values = grouped.into_values();
        if let Some((stream, _)) = values.next() {
            return Ok(stream);
        }
        return Err(ConsumerError::Configuration(
            "No JetStream stream configured for subjects".to_string(),
        ));
    }

    let streams = grouped.keys().cloned().collect::<Vec<_>>().join(", ");
    Err(ConsumerError::Configuration(format!(
        "Subjects span multiple JetStream streams ({streams}); use build_nats_consumer_tasks"
    )))
}

fn sanitize_task_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn durable_consumer_name(group: &str, stream_name: &str, subjects: &[String]) -> String {
    let mut sorted_subjects = subjects.to_vec();
    sorted_subjects.sort();
    sorted_subjects.dedup();

    let mut hash = 0xcbf29ce484222325u64;
    for part in std::iter::once(stream_name)
        .chain(std::iter::once(group))
        .chain(sorted_subjects.iter().map(String::as_str))
    {
        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    let base = format!(
        "consumer_{}_{}_{}",
        sanitize_task_part(group),
        sanitize_task_part(stream_name),
        hash
    );
    base.chars().take(96).collect()
}

#[async_trait::async_trait]
impl MessageFetcher for NatsMessageFetcher {
    async fn fetch(&mut self) -> Result<Option<Message>, ConsumerError> {
        match self.stream.next().await {
            Some(Ok(msg)) => {
                let message = self.decode_message(&msg)?;

                Ok(Some(message))
            }
            None => Ok(None),
            Some(Err(e)) => {
                if e.to_string().contains("timeout") {
                    Ok(None)
                } else {
                    Err(ConsumerError::Connection(e.to_string()))
                }
            }
        }
    }
}

impl NatsMessageFetcher {
    /// 解码 NATS 消息
    fn decode_message(&self, msg: &NatsMessage) -> Result<Message, ConsumerError> {
        let payload = msg.payload.to_vec();
        let subject = msg.subject.as_str();

        // 提取消息头
        let mut headers = HashMap::new();
        if let Some(nats_headers) = &msg.headers {
            for (key, values) in nats_headers.iter() {
                if let Some(value) = values.first() {
                    headers.insert(key.to_string(), value.to_string());
                }
            }
        }

        // 提取消息键
        let key = headers.get("x-message-key").cloned();

        // 提取消息 ID（从 headers 或生成）
        let message_id = headers
            .get("x-message-id")
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // 从 headers 中提取并构建 Context
        let ctx = self.build_context_from_headers(&headers);

        // 提取内容类型
        let content_type = headers
            .get("content-type")
            .and_then(|v| ContentType::from_str(v))
            .unwrap_or(ContentType::Json);
        let retry_count = retry_count_from_headers(&headers).max(nats_delivery_retry_count(msg));

        let message_context = MessageContext {
            ctx,
            message_id,
            topic: subject.to_string(),
            partition: 0, // NATS 不支持分区
            offset: 0,    // NATS 不支持 offset
            key,
            headers,
            started_at: std::time::Instant::now(),
            retry_count,
            metadata: HashMap::new(),
        };

        let message = Message::new(payload, content_type, message_context);
        if self.manual_ack {
            Ok(message.with_ack_handle(Arc::new(NatsMessageAck {
                message: msg.clone(),
            })))
        } else {
            Ok(message)
        }
    }

    /// 从 NATS 消息头中构建 Context（Context 透传）
    fn build_context_from_headers(
        &self,
        headers: &HashMap<String, String>,
    ) -> flare_core_base::context::Ctx {
        use flare_core_base::context::keys;
        use flare_core_base::context::{ActorContext, ActorType, Context};

        // 提取 request_id
        let request_id = headers
            .get(keys::REQUEST_ID)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut ctx = Context::with_request_id(request_id);

        // Trace ID（如果存在则复用）
        if let Some(trace_id) = headers.get(keys::TRACE_ID).filter(|s| !s.is_empty()) {
            ctx = ctx.with_trace_id(trace_id);
        }

        // Tenant ID
        if let Some(tenant_id) = headers.get(keys::TENANT_ID).filter(|s| !s.is_empty()) {
            ctx = ctx.with_tenant_id(tenant_id);
        }

        // User ID
        if let Some(user_id) = headers.get(keys::USER_ID).filter(|s| !s.is_empty()) {
            ctx = ctx.with_user_id(user_id);
        }

        // Device ID
        if let Some(device_id) = headers.get(keys::DEVICE_ID).filter(|s| !s.is_empty()) {
            ctx = ctx.with_device_id(device_id);
        }

        // Platform
        if let Some(platform) = headers.get(keys::PLATFORM).filter(|s| !s.is_empty()) {
            ctx = ctx.with_platform(platform);
        }

        // Session ID
        if let Some(session_id) = headers.get(keys::SESSION_ID).filter(|s| !s.is_empty()) {
            ctx = ctx.with_session_id(session_id);
        }

        // Actor 信息
        if let Some(actor_id) = headers.get(keys::ACTOR_ID).filter(|s| !s.is_empty()) {
            let actor_type = headers
                .get(keys::ACTOR_TYPE)
                .and_then(|s| s.parse::<i32>().ok())
                .map(ActorType::from)
                .unwrap_or(ActorType::Unspecified);

            let mut actor = ActorContext::new(actor_id).with_type(actor_type);

            // Roles（逗号分隔）
            if let Some(roles_str) = headers.get("x-actor-roles").filter(|s| !s.is_empty()) {
                for role in roles_str.split(',') {
                    let role = role.trim();
                    if !role.is_empty() {
                        actor = actor.with_role(role);
                    }
                }
            }

            // Attributes（前缀匹配）
            let prefix = "x-actor-attr-";
            for (key, value) in headers.iter() {
                if let Some(attr_key) = key.strip_prefix(prefix)
                    && !value.is_empty()
                {
                    actor = actor.with_attribute(attr_key, value);
                }
            }

            ctx = ctx.with_actor(actor);
        }

        Arc::new(ctx)
    }
}

fn nats_delivery_retry_count(msg: &NatsMessage) -> u32 {
    msg.info()
        .ok()
        .map(|info| delivery_attempt_retry_count(info.delivered))
        .unwrap_or_default()
}

fn delivery_attempt_retry_count(delivered: i64) -> u32 {
    let redeliveries = delivered.saturating_sub(1);
    if redeliveries <= 0 {
        return 0;
    }
    redeliveries.min(u32::MAX as i64) as u32
}

struct NatsMessageAck {
    message: NatsMessage,
}

fn record_nats_process_ack(
    operation: &'static str,
    started_at: Instant,
    result: &Result<(), ConsumerError>,
) {
    let outcome = if result.is_ok() { "success" } else { "error" };
    record_process_ack("jetstream", operation, outcome, started_at.elapsed());

    match result {
        Ok(()) => {
            tracing::trace!(
                backend = "jetstream",
                operation,
                "JetStream consumer process ack operation completed"
            );
        }
        Err(error) => {
            tracing::warn!(
                backend = "jetstream",
                operation,
                error = %error,
                "JetStream consumer process ack operation failed"
            );
        }
    }
}

#[async_trait::async_trait]
impl MessageAck for NatsMessageAck {
    async fn ack(&self) -> Result<(), ConsumerError> {
        let started_at = Instant::now();
        let result = self
            .message
            .ack()
            .await
            .map_err(|e| ConsumerError::Connection(e.to_string()));
        record_nats_process_ack("ack", started_at, &result);
        result
    }

    async fn nack(&self) -> Result<(), ConsumerError> {
        let started_at = Instant::now();
        let result = self
            .message
            .ack_with(jetstream::AckKind::Nak(None))
            .await
            .map_err(|e| ConsumerError::Connection(e.to_string()));
        record_nats_process_ack("nack", started_at, &result);
        result
    }

    async fn term(&self) -> Result<(), ConsumerError> {
        let started_at = Instant::now();
        let result = self
            .message
            .ack_with(jetstream::AckKind::Term)
            .await
            .map_err(|e| ConsumerError::Connection(e.to_string()));
        record_nats_process_ack("term", started_at, &result);
        result
    }
}

/// NATS JetStream 消费者运行时
///
/// 封装了 NATS JetStream 特定的消费者逻辑
pub struct NatsConsumerRuntime {
    config: ConsumerConfig,
    dispatcher: Arc<dyn crate::mq::consumer::Dispatcher>,
}

impl NatsConsumerRuntime {
    /// 创建新的 NATS JetStream 消费者运行时
    pub fn new(
        config: ConsumerConfig,
        dispatcher: Arc<dyn crate::mq::consumer::Dispatcher>,
    ) -> Self {
        Self { config, dispatcher }
    }

    /// 启动 NATS JetStream 消费者
    pub async fn start<C>(&self, nats_config: C) -> Result<(), ConsumerError>
    where
        C: NatsConsumerConfig + Send + Sync + 'static,
    {
        let topics = self.dispatcher.topics();
        if topics.is_empty() {
            return Err(ConsumerError::Configuration(
                "No topics registered in dispatcher".to_string(),
            ));
        }

        tracing::info!(subjects = ?topics, "Starting NATS JetStream consumer");

        let fetcher = NatsMessageFetcher::new(&nats_config, topics).await?;
        let runtime =
            crate::mq::consumer::ConsumerRuntime::new(self.config.clone(), self.dispatcher.clone());

        let mut fetcher = fetcher;
        runtime.run(&mut fetcher).await
    }
}

/// NATS JetStream 消费者构建器
///
/// 提供便捷的构建方式来创建 NATS JetStream 消费者
pub struct NatsConsumerBuilder {
    config: ConsumerConfig,
    dispatcher: crate::mq::consumer::TopicDispatcher,
}

impl NatsConsumerBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            config: ConsumerConfig::default(),
            dispatcher: crate::mq::consumer::TopicDispatcher::new(),
        }
    }

    /// 设置配置
    pub fn with_config(mut self, config: ConsumerConfig) -> Self {
        self.config = config;
        self
    }

    /// 注册 Handler
    pub fn register_handler<H>(
        mut self,
        handler: H,
        subjects: Vec<String>,
    ) -> Result<Self, ConsumerError>
    where
        H: crate::mq::consumer::MessageHandler + 'static,
    {
        let handler_arc: Arc<dyn crate::mq::consumer::MessageHandler> = Arc::new(handler);

        for subject in subjects {
            self.dispatcher.register(subject, handler_arc.clone())?;
        }

        Ok(self)
    }

    /// 构建运行时
    pub fn build(self) -> NatsConsumerRuntime {
        NatsConsumerRuntime::new(self.config, Arc::new(self.dispatcher))
    }
}

impl Default for NatsConsumerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{durable_consumer_name, sanitize_task_part};

    /// 陈旧 durable 的识别只能靠 `consumer_<组>_<流>_` 前缀，
    /// 而且必须**只**匹配同组同流——误删别的消费组的 durable
    /// 会让那个组从头重放整个流，比留着孤儿严重得多。
    #[test]
    fn stale_durable_prefix_matches_same_group_and_stream_only() {
        let mine = durable_consumer_name(
            "conversation-read-receipt",
            "FLARE_MESSAGE",
            &["flare.im.message.events".to_string()],
        );
        let after_adding_subject = durable_consumer_name(
            "conversation-read-receipt",
            "FLARE_MESSAGE",
            &[
                "flare.im.message.events".to_string(),
                "flare.im.message.storage".to_string(),
            ],
        );
        let other_group = durable_consumer_name(
            "storage-writer",
            "FLARE_MESSAGE",
            &["flare.im.message.events".to_string()],
        );
        let other_stream = durable_consumer_name(
            "conversation-read-receipt",
            "FLARE_PUSH",
            &["flare.im.message.events".to_string()],
        );

        let prefix = format!(
            "consumer_{}_{}_",
            sanitize_task_part("conversation-read-receipt"),
            sanitize_task_part("FLARE_MESSAGE")
        );

        // 加了 subject 就换名字——这正是孤儿的来源
        assert_ne!(mine, after_adding_subject, "改 subject 必须产生新 durable");
        // 同组同流的旧 durable 要被认出来
        assert!(mine.starts_with(&prefix));
        assert!(after_adding_subject.starts_with(&prefix));
        // 别的组、别的流一律不许碰
        assert!(!other_group.starts_with(&prefix), "不能误伤别的消费组");
        assert!(!other_stream.starts_with(&prefix), "不能误伤别的流");
    }
    use super::delivery_attempt_retry_count;

    #[test]
    fn delivery_attempt_retry_count_treats_first_delivery_as_zero_retries() {
        assert_eq!(delivery_attempt_retry_count(0), 0);
        assert_eq!(delivery_attempt_retry_count(1), 0);
    }

    #[test]
    fn delivery_attempt_retry_count_maps_redelivery_attempts() {
        assert_eq!(delivery_attempt_retry_count(2), 1);
        assert_eq!(delivery_attempt_retry_count(16), 15);
    }
}
