//! NATS JetStream 生产者实现
//!
//! 实现基于 NATS JetStream 的消息生产，支持 Context 透传。

use std::collections::HashMap;
use std::time::Duration;

use async_nats::HeaderMap;
use async_nats::jetstream;

use super::config::{NatsProducerConfig, NatsStreamSpec, resolve_subject_stream};
use crate::mq::producer::{Producer, ProducerError, ProducerMessage};

/// NATS JetStream 生产者
pub struct NatsProducer {
    context: jetstream::Context,
    stream_specs: Vec<NatsStreamSpec>,
    retries: u32,
    retry_backoff: Duration,
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

        let mut context = jetstream::new(client);
        context.set_timeout(Duration::from_millis(config.timeout_ms()));
        let stream_specs = config.stream_specs();
        if stream_specs.is_empty() {
            return Err(ProducerError::Configuration(
                "JetStream stream specs cannot be empty".to_string(),
            ));
        }

        for spec in &stream_specs {
            ensure_stream(&context, spec).await?;
        }

        Ok(Self {
            context,
            stream_specs,
            retries: config.retries(),
            retry_backoff: Duration::from_millis(config.retry_backoff_ms()),
        })
    }

    /// 添加 Context 到 NATS 消息头
    fn add_context_to_headers(&self, headers: &mut HeaderMap, ctx: &flare_core_base::context::Ctx) {
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
            headers.insert(keys::ACTOR_ID, actor.actor_id());
            headers.insert(keys::ACTOR_TYPE, (actor.actor_type as i32).to_string());

            // Roles（逗号分隔）
            if !actor.roles().is_empty() {
                let roles = actor.roles().join(",");
                headers.insert("x-actor-roles", roles);
            }

            // Attributes（前缀匹配）
            for (key, value) in actor.attributes() {
                headers.insert(format!("x-actor-attr-{}", key), value.as_ref());
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
        let _stream = resolve_subject_stream(&self.stream_specs, subject).ok_or_else(|| {
            ProducerError::Configuration(format!(
                "No JetStream stream configured for subject: {}",
                subject
            ))
        })?;

        let mut publish = jetstream::context::Publish::build().payload(payload.into());

        // 添加消息 ID。优先使用业务传入的幂等 ID，保证 JetStream 去重跨重试稳定。
        let message_id = headers
            .as_ref()
            .and_then(|h| {
                h.get("x-message-id")
                    .or_else(|| h.get("message_id"))
                    .or_else(|| h.get("event_id"))
                    .or_else(|| h.get("request_id"))
            })
            .cloned()
            .unwrap_or_else(|| self.generate_message_id());
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

        // 发送消息并等待 JetStream ack，只有真正落到目标 stream 后才返回成功。
        // 使用稳定 message_id 后，重试不会造成重复落盘。
        let mut attempt = 0u32;
        let ack = loop {
            match self
                .context
                .send_publish(subject.to_string(), publish.clone())
                .await
            {
                Ok(pending_ack) => match pending_ack.await {
                    Ok(ack) => break ack,
                    Err(err) => {
                        if attempt >= self.retries {
                            return Err(ProducerError::Send(err.to_string()));
                        }
                        attempt += 1;
                        tracing::warn!(
                            subject = %subject,
                            message_id = %message_id,
                            attempt,
                            max_retries = self.retries,
                            error = %err,
                            "NATS publish ack failed, retrying"
                        );
                    }
                },
                Err(err) => {
                    if attempt >= self.retries {
                        return Err(ProducerError::Send(err.to_string()));
                    }
                    attempt += 1;
                    tracing::warn!(
                        subject = %subject,
                        message_id = %message_id,
                        attempt,
                        max_retries = self.retries,
                        error = %err,
                        "NATS publish request failed, retrying"
                    );
                }
            }

            tokio::time::sleep(self.retry_backoff.saturating_mul(attempt)).await;
        };

        tracing::trace!(
            subject = %subject,
            stream = %ack.stream,
            sequence = ack.sequence,
            duplicate = ack.duplicate,
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

async fn ensure_stream(
    context: &jetstream::Context,
    spec: &NatsStreamSpec,
) -> Result<(), ProducerError> {
    let stream_name = spec.name.as_str();
    let subjects = spec.subjects.clone();
    if subjects.is_empty() {
        return Err(ProducerError::Configuration(
            "JetStream stream subjects cannot be empty".to_string(),
        ));
    }

    let config = jetstream::stream::Config {
        name: stream_name.to_string(),
        subjects,
        retention: jetstream::stream::RetentionPolicy::Limits,
        max_age: spec.max_age,
        storage: jetstream::stream::StorageType::File,
        num_replicas: spec.num_replicas,
        duplicate_window: spec.duplicate_window,
        ..Default::default()
    };

    context
        .create_or_update_stream(config)
        .await
        .map(|_| ())
        .map_err(|e| ProducerError::Configuration(e.to_string()))
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
