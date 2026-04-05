//! NATS JetStream 生产者实现
//!
//! 实现基于 NATS JetStream 的消息生产，支持 Context 透传。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_nats::HeaderMap;
use async_nats::jetstream;
use async_nats::jetstream::context::Publish;

use super::config::NatsProducerConfig;
use crate::mq::producer::{Producer, ProducerError, ProducerMessage};

/// NATS JetStream 生产者
pub struct NatsProducer {
    context: jetstream::Context,
    config: crate::mq::producer::ProducerConfig,
}

impl NatsProducer {
    /// 创建新的 NATS JetStream 生产者
    pub async fn new<C>(config: &C) -> Result<Self, ProducerError>
    where
        C: NatsProducerConfig + Send + Sync,
    {
        let client = async_nats::connect(&config.nats_url())
            .await
            .map_err(|e| ProducerError::Connection(e.to_string()))?;

        let context =
            jetstream::new(client).with_timeout(Duration::from_millis(config.timeout_ms()));

        let producer_config = crate::mq::producer::ProducerConfig {
            timeout_ms: config.timeout_ms(),
            enable_idempotence: true, // JetStream 默认支持
            compression_type: "none".to_string(),
            batch_size: 0, // NATS 不支持批处理
            linger_ms: 0,
            retries: config.retries(),
            retry_backoff_ms: config.retry_backoff_ms(),
        };

        Ok(Self {
            context,
            config: producer_config,
        })
    }

    /// 添加 Context 到 NATS 消息头
    fn add_context_to_headers(&self, headers: &mut HeaderMap, ctx: &flare_core_base::context::Ctx) {
        use flare_core_base::context::Context;
        use flare_core_base::context::keys;

        // Trace ID
        headers.insert(keys::TRACE_ID, ctx.trace_id());

        // Request ID
        headers.insert(keys::REQUEST_ID, ctx.request_id());

        // Tenant ID
        if let Some(tenant_id) = ctx.tenant_id() {
            headers.insert(keys::TENANT_ID, tenant_id);
        }

        // User ID
        if let Some(user_id) = ctx.user_id() {
            headers.insert(keys::USER_ID, user_id);
        }

        // Device ID
        if let Some(device_id) = ctx.device_id() {
            headers.insert(keys::DEVICE_ID, device_id);
        }

        // Platform
        if let Some(platform) = ctx.platform() {
            headers.insert(keys::PLATFORM, platform);
        }

        // Session ID
        if let Some(session_id) = ctx.session_id() {
            headers.insert(keys::SESSION_ID, session_id);
        }

        // Actor 信息
        if let Some(actor) = ctx.actor() {
            headers.insert(keys::ACTOR_ID, actor.id());
            headers.insert(keys::ACTOR_TYPE, (actor.type_() as i32).to_string());

            // Roles（逗号分隔）
            if !actor.roles().is_empty() {
                let roles = actor.roles().join(",");
                headers.insert("x-actor-roles", roles);
            }

            // Attributes（前缀匹配）
            for (key, value) in actor.attributes() {
                headers.insert(format!("x-actor-attr-{}", key), value);
            }
        }
    }

    /// 生成消息 ID
    fn generate_message_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[async_trait::async_trait]
impl Producer for NatsProducer {
    async fn send(
        &self,
        ctx: &flare_core_base::context::Ctx,
        subject: &str,
        key: Option<&str>,
        payload: Vec<u8>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<(), ProducerError> {
        // NATS 使用 subject 而不是 topic
        let mut publish = self.context.publish(subject, payload.into());

        // 添加消息 ID
        let message_id = self.generate_message_id();
        publish = publish.message_id(message_id.clone());

        // 准备消息头
        let mut header_map = HeaderMap::new();

        // 如果提供了自定义 headers，先添加
        if let Some(h) = headers {
            for (k, v) in h {
                header_map.insert(k, v);
            }
        }

        // 添加 Context 到消息头
        self.add_context_to_headers(&mut header_map, ctx);

        // 如果有 key，添加到 header
        if let Some(k) = key {
            header_map.insert("x-message-key", k);
        }

        publish = publish.headers(header_map);

        // 发送消息
        publish
            .await
            .map_err(|e| ProducerError::Send(e.to_string()))?;

        tracing::debug!(
            subject = %subject,
            message_id = %message_id,
            trace_id = %ctx.trace_id(),
            "Message sent to NATS successfully"
        );

        Ok(())
    }

    async fn send_batch(
        &self,
        ctx: &flare_core_base::context::Ctx,
        messages: Vec<ProducerMessage>,
    ) -> Result<(), ProducerError> {
        for msg in messages {
            self.send(
                ctx,
                &msg.topic, // NATS 使用 subject
                msg.key.as_deref(),
                msg.payload,
                Some(msg.headers),
            )
            .await?;
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "nats_jetstream_producer"
    }
}

/// NATS JetStream 生产者构建器
pub struct NatsProducerBuilder {
    config: crate::mq::producer::ProducerConfig,
}

impl NatsProducerBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            config: crate::mq::producer::ProducerConfig::default(),
        }
    }

    /// 设置配置
    pub fn with_config(mut self, config: crate::mq::producer::ProducerConfig) -> Self {
        self.config = config;
        self
    }

    /// 构建 NATS JetStream 生产者
    pub async fn build<C>(self, nats_config: &C) -> Result<NatsProducer, ProducerError>
    where
        C: NatsProducerConfig + Send + Sync,
    {
        NatsProducer::new(nats_config).await
    }
}

impl Default for NatsProducerBuilder {
    fn default() -> Self {
        Self::new()
    }
}
