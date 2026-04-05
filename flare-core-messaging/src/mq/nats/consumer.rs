//! NATS JetStream 消费者实现
//!
//! 实现基于 NATS JetStream 的消息消费，支持并发消费、Context 透传等特性。

use std::collections::HashMap;
use std::sync::Arc;

use async_nats::Message as NatsMessage;
use async_nats::jetstream;
use async_nats::jetstream::consumer::pull::Config as PullConfig;
use async_nats::jetstream::consumer::pull::Stream as PullStream;

use super::config::NatsConsumerConfig;
use crate::mq::consumer::{
    ConsumerConfig, ConsumerError, ContentType, Message, MessageContext, MessageFetcher,
};

/// NATS JetStream 消息获取器
pub struct NatsMessageFetcher {
    stream: PullStream,
    subjects: Vec<String>,
    manual_ack: bool,
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

        // 创建或获取 stream
        let stream_name = format!("stream_{}", config.consumer_group());
        let stream = jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: subjects.clone(),
            retention: jetstream::stream::RetentionPolicy::Limits,
            max_age: std::time::Duration::from_secs(7 * 24 * 3600),
            storage: jetstream::stream::StorageType::File,
            ..Default::default()
        };

        let _ = context.create_stream(stream).await;

        // 创建 consumer
        let consumer_name = if config.enable_durable() {
            format!("consumer_{}", config.consumer_group())
        } else {
            "ephemeral_consumer".to_string()
        };

        let consumer_config = PullConfig {
            name: Some(consumer_name),
            durable_name: if config.enable_durable() {
                Some(format!("consumer_{}", config.consumer_group()))
            } else {
                None
            },
            ack_policy: if manual_ack {
                jetstream::consumer::AckPolicy::Explicit
            } else {
                jetstream::consumer::AckPolicy::None
            },
            batch_size: config.batch_size(),
            batch_timeout: std::time::Duration::from_millis(config.batch_timeout_ms()),
            ..Default::default()
        };

        let stream = context
            .create_consumer(stream_name.clone(), consumer_config)
            .await
            .map_err(|e| ConsumerError::Configuration(e.to_string()))?;

        let fetcher = Self {
            stream,
            subjects,
            manual_ack,
        };

        tracing::info!(subjects = ?subjects, "Subscribed to NATS subjects");
        Ok(fetcher)
    }
}

#[async_trait::async_trait]
impl MessageFetcher for NatsMessageFetcher {
    async fn fetch(&mut self) -> Result<Option<Message>, ConsumerError> {
        match self.stream.next().await {
            Ok(Some(msg)) => {
                let message = self.decode_message(&msg)?;

                // 手动确认
                if self.manual_ack {
                    if let Err(e) = msg.ack().await {
                        tracing::warn!(error = %e, "Failed to ack message");
                    }
                }

                Ok(Some(message))
            }
            Ok(None) => Ok(None),
            Err(e) => {
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
                    headers.insert(key.clone(), value.clone());
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

        let message_context = MessageContext {
            ctx,
            message_id,
            topic: subject.to_string(),
            partition: 0, // NATS 不支持分区
            offset: 0,    // NATS 不支持 offset
            key,
            headers,
            started_at: std::time::Instant::now(),
            retry_count: 0,
            metadata: HashMap::new(),
        };

        Ok(Message::new(payload, content_type, message_context))
    }

    /// 从 NATS 消息头中构建 Context（Context 透传）
    fn build_context_from_headers(
        &self,
        headers: &HashMap<String, String>,
    ) -> flare_core_base::context::Ctx {
        use flare_core_base::context::keys;
        use flare_core_base::context::{ActorContext, ActorType, Context, Ctx};

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
                if let Some(attr_key) = key.strip_prefix(prefix) {
                    if !value.is_empty() {
                        actor = actor.with_attribute(attr_key, value);
                    }
                }
            }

            ctx = ctx.with_actor(actor);
        }

        Arc::new(ctx)
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
