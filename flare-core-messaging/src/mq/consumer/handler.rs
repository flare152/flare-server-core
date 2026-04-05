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
