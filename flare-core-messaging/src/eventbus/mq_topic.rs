//! MQ 事件总线实现：基于 [crate::mq] 的统一事件总线实现
//!
//! 提供基于 Kafka/NATS 等消息队列的事件发布和订阅功能

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::eventbus::constants::{
    EVENT_ENVELOPE_CONTENT_TYPE, EVENT_ENVELOPE_PROTO_CONTENT_TYPE, HEADER_CONTENT_TYPE,
};
use crate::eventbus::envelope::EventEnvelope;
use crate::eventbus::event_bus::{EventBus, EventHandler, EventPublisher, EventSubscriber};
use crate::mq::consumer::dispatcher::Dispatcher;
use crate::mq::consumer::{
    ConsumerConfig, ConsumerError, ConsumerRuntime, ContentType, Message, MessageFetcher,
    MessageHandler, MessageResult, TopicDispatcher,
};
use crate::mq::producer::Producer;
use flare_core_base::context::Ctx;
use flare_core_base::error::{FlareError, Result};
use tracing::debug;

// =============================================================================
// 辅助函数
// =============================================================================

/// 从消息中反序列化 EventEnvelope（支持 JSON 与 Protobuf；Protobuf 需启用 `proto` feature）。
#[inline]
fn event_envelope_from_message(
    message: &Message,
) -> std::result::Result<EventEnvelope, ConsumerError> {
    match &message.content_type {
        ContentType::Json => message
            .to_json::<EventEnvelope>()
            .map_err(|e| ConsumerError::Deserialization(e.to_string())),
        ContentType::Protobuf => {
            #[cfg(feature = "proto")]
            {
                EventEnvelope::from_proto_bytes(&message.payload)
                    .map_err(|e| ConsumerError::Deserialization(e.to_string()))
            }
            #[cfg(not(feature = "proto"))]
            {
                Err(ConsumerError::Deserialization(
                    "received protobuf EventEnvelope but flare-server-core compiled without proto feature"
                        .to_string(),
                ))
            }
        }
        other => Err(ConsumerError::Deserialization(format!(
            "topic event bus expects EventEnvelope JSON({}) or Protobuf({}), got {:?}",
            EVENT_ENVELOPE_CONTENT_TYPE, EVENT_ENVELOPE_PROTO_CONTENT_TYPE, other
        ))),
    }
}

/// 创建发布事件时的标准 Headers
#[inline]
fn envelope_publish_headers() -> HashMap<String, String> {
    let mut h = HashMap::with_capacity(2);
    h.insert(HEADER_CONTENT_TYPE.to_string(), {
        #[cfg(feature = "proto")]
        {
            EVENT_ENVELOPE_PROTO_CONTENT_TYPE.to_string()
        }
        #[cfg(not(feature = "proto"))]
        {
            EVENT_ENVELOPE_CONTENT_TYPE.to_string()
        }
    });
    h
}

// =============================================================================
// MqEventBus - MQ 事件总线实现
// =============================================================================

/// MQ 事件总线实现
///
/// 基于 [crate::mq::producer::Producer] 和 [crate::mq::consumer] 实现
/// 具体的 Broker 由 Producer/Consumer 实现决定（Kafka/NATS 等）
pub struct MqEventBus {
    producer: Arc<dyn Producer>,
}

impl MqEventBus {
    /// 创建新的 MQ 事件总线
    ///
    /// # 参数
    /// - `producer`: MQ 生产者实例
    ///
    /// # 返回
    /// - `Arc<Self>`: 事件总线的 Arc 包装
    pub fn new(producer: Arc<dyn Producer>) -> Arc<Self> {
        Arc::new(Self { producer })
    }
}

// =============================================================================
// EventPublisher 实现
// =============================================================================

impl EventPublisher for MqEventBus {
    async fn publish(&self, ctx: &Ctx, topic: &str, envelope: &EventEnvelope) -> Result<()> {
        let payload = {
            #[cfg(feature = "proto")]
            {
                envelope.to_proto_bytes()?
            }
            #[cfg(not(feature = "proto"))]
            {
                envelope.to_json_bytes()?
            }
        };
        let headers = envelope_publish_headers();
        self.producer
            .send(
                ctx,
                topic,
                Some(envelope.partition_key()),
                payload,
                Some(headers),
            )
            .await
            .map_err(|e| FlareError::message_send_failed(e.to_string()))?;
        debug!(
            topic = %topic,
            event_id = %envelope.event_id,
            "Event published via MQ producer"
        );
        Ok(())
    }
}

// =============================================================================
// EventSubscriber 实现
// =============================================================================

impl EventSubscriber for MqEventBus {
    async fn subscribe(&self, topic: &str, handler: Arc<dyn EventHandler>) -> Result<()> {
        self.subscribe_with_consumer_group(topic, None, handler)
            .await
    }

    async fn subscribe_with_consumer_group(
        &self,
        topic: &str,
        consumer_group: Option<&str>,
        handler: Arc<dyn EventHandler>,
    ) -> Result<()> {
        let wrapped: Arc<dyn MessageHandler> = Arc::new(MqEventHandler::new(handler.clone()));

        let mut dispatcher = TopicDispatcher::new();
        crate::mq::consumer::dispatcher::Dispatcher::register(
            &mut dispatcher,
            topic.to_string(),
            wrapped,
        )
        .map_err(|e| FlareError::general_error(e.to_string()))?;

        debug!(
            topic = %topic,
            consumer_group = ?consumer_group,
            handler = %handler.name(),
            "Event handler registered (consumer_group applies when building KafkaMessageFetcher / ConsumerConfig)"
        );
        Ok(())
    }
}

impl EventBus for MqEventBus {}

// =============================================================================
// MqEventHandler - MQ 事件处理器适配器
// =============================================================================

/// 将 [EventHandler] 适配为 [crate::mq::consumer::MessageHandler]
pub struct MqEventHandler {
    inner: Arc<dyn EventHandler>,
}

impl MqEventHandler {
    pub fn new(inner: Arc<dyn EventHandler>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl MessageHandler for MqEventHandler {
    async fn handle(&self, message: Message) -> std::result::Result<MessageResult, ConsumerError> {
        let envelope = event_envelope_from_message(&message)?;
        let ctx = &message.context.ctx;
        self.inner
            .handle(ctx, envelope)
            .await
            .map_err(|e| ConsumerError::Handler(e.to_string()))?;
        Ok(MessageResult::Ack)
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

// =============================================================================
// MqEventBusRuntime - MQ 事件总线运行时
// =============================================================================

/// MQ 事件总线运行时
///
/// 管理消费者的生命周期，支持优雅停机
pub struct MqEventBusRuntime {
    dispatcher: Arc<dyn Dispatcher>,
    config: ConsumerConfig,
}

impl MqEventBusRuntime {
    /// 创建新的运行时
    ///
    /// # 参数
    /// - `dispatcher`: 主题分发器
    /// - `config`: 消费者配置
    ///
    /// # 返回
    /// - `Self`: 运行时实例
    pub fn new(dispatcher: Arc<dyn Dispatcher>, config: ConsumerConfig) -> Self {
        Self { dispatcher, config }
    }

    /// 运行消费者
    ///
    /// # 参数
    /// - `fetcher`: 消息获取器
    ///
    /// # 返回
    /// - `Result<()>`: 运行成功或失败
    pub async fn run<MF>(self, fetcher: MF) -> Result<()>
    where
        MF: MessageFetcher + Send + 'static,
    {
        let runtime = ConsumerRuntime::new(self.config, self.dispatcher);
        let mut fetcher = fetcher;
        runtime
            .run(&mut fetcher)
            .await
            .map_err(|e| FlareError::general_error(e.to_string()))?;
        Ok(())
    }
}

// =============================================================================
// 辅助函数
// =============================================================================

/// 注册事件处理器到指定主题
///
/// # 参数
/// - `handler`: 事件处理器
/// - `topics`: 要订阅的主题列表
///
/// # 返回
/// - `Result<TopicDispatcher>`: 主题分发器
pub fn register_event_handler<H, I>(
    handler: Arc<H>,
    topics: I,
) -> std::result::Result<TopicDispatcher, ConsumerError>
where
    H: EventHandler + 'static,
    I: IntoIterator<Item = String>,
{
    let mut dispatcher = TopicDispatcher::new();
    let wrapped: Arc<dyn MessageHandler> = Arc::new(MqEventHandler::new(handler));
    for topic in topics {
        crate::mq::consumer::dispatcher::Dispatcher::register(
            &mut dispatcher,
            topic,
            wrapped.clone(),
        )?;
    }
    Ok(dispatcher)
}

/// 运行事件消费者
///
/// # 参数
/// - `fetcher`: 消息获取器
/// - `dispatcher`: 主题分发器
/// - `config`: 消费者配置
///
/// # 返回
/// - `Result<()>`: 运行成功或失败
pub async fn run_event_consumer<MF>(
    fetcher: MF,
    dispatcher: Arc<dyn Dispatcher>,
    config: ConsumerConfig,
) -> Result<()>
where
    MF: MessageFetcher + Send + 'static,
{
    let runtime = MqEventBusRuntime::new(dispatcher, config);
    runtime.run(fetcher).await
}

// =============================================================================
// 向后兼容的别名（保留旧 API）
// =============================================================================

/// 旧版 TopicEnvelopeHandler 的别名
#[deprecated(note = "Use Arc<dyn EventHandler> instead")]
pub type TopicEnvelopeHandler = Arc<dyn EventHandler>;

/// 旧版 TopicEnvelopeMessageHandler 的别名
#[deprecated(note = "Use MqEventHandler instead")]
pub type TopicEnvelopeMessageHandler = MqEventHandler;

/// 旧版 register_topic_envelope_dispatcher 的别名
#[deprecated(note = "Use register_event_handler instead")]
pub fn register_topic_envelope_dispatcher<H, I>(
    handler: Arc<H>,
    topics: I,
) -> std::result::Result<TopicDispatcher, ConsumerError>
where
    H: EventHandler + 'static,
    I: IntoIterator<Item = String>,
{
    register_event_handler(handler, topics)
}

/// 旧版 run_topic_event_consumer 的别名
#[deprecated(note = "Use run_event_consumer instead")]
pub async fn run_topic_event_consumer<MF>(
    fetcher: MF,
    dispatcher: Arc<dyn Dispatcher>,
    config: ConsumerConfig,
) -> Result<()>
where
    MF: MessageFetcher + Send + 'static,
{
    run_event_consumer(fetcher, dispatcher, config).await
}
