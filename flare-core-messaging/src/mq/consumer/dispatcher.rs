//! MQ 消息分发器
//!
//! 按 topic 路由消息到对应的处理器。
//!
//! Kafka **消费者组（`group.id`）** 由 [crate::mq::kafka::KafkaMessageFetcher] /
//! [crate::mq::consumer::ConsumerConfig::kafka_consumer_group_override] 与 `KafkaConsumerConfig` 决定，
//! 与 topic 路由正交（同一组可订阅多 topic，由本分发器按 topic 选 handler）。

use std::collections::HashMap;
use std::sync::Arc;

use super::handler::MessageHandler;
use super::types::{ConsumerError, Message, MessageResult};

/// 消息分发器 Trait
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    /// 分发消息
    async fn dispatch(&self, message: Message) -> Result<MessageResult, ConsumerError>;

    /// 注册处理器
    fn register(
        &mut self,
        topic: String,
        handler: Arc<dyn MessageHandler>,
    ) -> Result<(), ConsumerError>;

    /// 获取所有注册的主题
    fn topics(&self) -> Vec<String>;
}

/// 基于 Topic 的分发器
pub struct TopicDispatcher {
    handlers: HashMap<String, Arc<dyn MessageHandler>>,
}

impl TopicDispatcher {
    /// 创建新的分发器
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// 查找匹配的处理器
    fn find_handler(&self, topic: &str) -> Option<Arc<dyn MessageHandler>> {
        // 精确匹配
        if let Some(handler) = self.handlers.get(topic) {
            return Some(handler.clone());
        }

        // 通配符匹配（简单实现）
        for (pattern, handler) in &self.handlers {
            if pattern.ends_with('*') {
                let prefix = &pattern[..pattern.len() - 1];
                if topic.starts_with(prefix) {
                    return Some(handler.clone());
                }
            }
        }

        None
    }
}

impl Default for TopicDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Dispatcher for TopicDispatcher {
    async fn dispatch(&self, message: Message) -> Result<MessageResult, ConsumerError> {
        let topic = message.context.topic.clone();

        match self.find_handler(&topic) {
            Some(handler) => {
                let handler_name = handler.name();
                tracing::debug!(topic = %topic, handler = %handler_name, "Dispatching message");
                handler.handle(message).await
            }
            None => {
                tracing::warn!(topic = %topic, "No handler found for topic");
                Err(ConsumerError::NoHandler(topic))
            }
        }
    }

    fn register(
        &mut self,
        topic: String,
        handler: Arc<dyn MessageHandler>,
    ) -> Result<(), ConsumerError> {
        if self.handlers.contains_key(&topic) {
            return Err(ConsumerError::Configuration(format!(
                "Handler already registered for topic: {}",
                topic
            )));
        }

        self.handlers.insert(topic, handler);
        Ok(())
    }

    fn topics(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}

/// 基于注册表的分发器
pub struct RegistryDispatcher {
    registry: Arc<super::handler::HandlerRegistry>,
}

impl RegistryDispatcher {
    /// 创建新的分发器
    pub fn new() -> Self {
        Self {
            registry: Arc::new(super::handler::HandlerRegistry::new()),
        }
    }

    /// 获取注册表
    pub fn registry(&self) -> Arc<super::handler::HandlerRegistry> {
        self.registry.clone()
    }
}

impl Default for RegistryDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Dispatcher for RegistryDispatcher {
    async fn dispatch(&self, message: Message) -> Result<MessageResult, ConsumerError> {
        let topic = message.context.topic.clone();

        // 使用 topic 作为 handler 名称
        match self.registry.get(&topic) {
            Some(handler) => {
                let handler_name = handler.name();
                tracing::debug!(topic = %topic, handler = %handler_name, "Dispatching message");
                handler.handle(message).await
            }
            None => {
                tracing::warn!(topic = %topic, "No handler found for topic");
                Err(ConsumerError::NoHandler(topic))
            }
        }
    }

    fn register(
        &mut self,
        topic: String,
        handler: Arc<dyn MessageHandler>,
    ) -> Result<(), ConsumerError> {
        let mut registry = Arc::try_unwrap(self.registry.clone()).map_err(|_| {
            ConsumerError::Configuration("Registry is shared, cannot mutate".to_string())
        })?;

        registry.register(topic.clone(), handler);
        self.registry = Arc::new(registry);
        Ok(())
    }

    fn topics(&self) -> Vec<String> {
        self.registry.list()
    }
}
