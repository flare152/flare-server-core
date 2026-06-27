//! MQ 消息处理器
//!
//! 定义消息处理接口和注册机制。

use std::collections::HashMap;
use std::sync::Arc;

use super::types::{ConsumerError, Message, MessageResult};

/// 消息处理器 Trait
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    /// 处理消息
    ///
    /// # 参数
    /// * `message` - 消息内容
    ///
    /// # 返回
    /// * `Result<MessageResult, ConsumerError>` - 处理结果
    async fn handle(&self, message: Message) -> Result<MessageResult, ConsumerError>;

    /// 批量处理消息。
    ///
    /// 默认实现保持向后兼容：逐条调用 [Self::handle]。高吞吐消费者可以覆盖此方法，
    /// 在一次事务或一次外部调用中完成批量处理。返回结果必须与输入消息一一对应。
    async fn handle_batch(
        &self,
        messages: Vec<Message>,
    ) -> Result<Vec<MessageResult>, ConsumerError> {
        let mut results = Vec::with_capacity(messages.len());
        for message in messages {
            results.push(self.handle(message).await?);
        }
        Ok(results)
    }

    /// 是否支持真正的原子批量处理。
    ///
    /// 默认关闭，避免普通逐条 handler 在批量中部分成功后整体 NACK，造成非幂等副作用重放。
    fn supports_batch(&self) -> bool {
        false
    }

    /// 获取处理器名称
    fn name(&self) -> &str;
}

/// Handler 注册表
pub struct HandlerRegistry {
    handlers: HashMap<String, Arc<dyn MessageHandler>>,
}

impl HandlerRegistry {
    /// 创建新的注册表
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// 注册处理器
    pub fn register(&mut self, name: String, handler: Arc<dyn MessageHandler>) {
        self.handlers.insert(name, handler);
    }

    /// 获取处理器
    pub fn get(&self, name: &str) -> Option<Arc<dyn MessageHandler>> {
        self.handlers.get(name).cloned()
    }

    /// 检查是否包含指定处理器
    pub fn contains(&self, name: &str) -> bool {
        self.handlers.contains_key(name)
    }

    /// 获取所有处理器名称
    pub fn list(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
