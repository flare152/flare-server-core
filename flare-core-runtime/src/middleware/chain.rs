//! 中间件链实现
//!
//! 管理所有中间件并按顺序执行

use super::Middleware;
use crate::error::MiddlewareError;
use std::sync::Arc;
use tracing::warn;

/// 中间件链
///
/// 管理所有中间件并按顺序执行
///
/// # 示例
///
/// ```rust
/// use flare_core_runtime::middleware::{MiddlewareChain, Middleware};
///
/// let mut chain = MiddlewareChain::new();
/// chain.add(Arc::new(MyMiddleware));
///
/// // 执行中间件
/// chain.before("task-1").await?;
/// // ... 执行任务 ...
/// chain.after("task-1", &result).await?;
/// ```
pub struct MiddlewareChain {
    middlewares: Vec<Arc<dyn Middleware>>,
}

impl MiddlewareChain {
    /// 创建新的中间件链
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
        }
    }

    /// 添加中间件
    ///
    /// # 参数
    ///
    /// * `middleware` - 要添加的中间件
    pub fn add(&mut self, middleware: Arc<dyn Middleware>) {
        let name = middleware.name().to_string();
        self.middlewares.push(middleware);
        tracing::info!(middleware_name = %name, "Middleware added");
    }

    /// 执行所有中间件的 before 钩子
    ///
    /// 按注册顺序执行，如果某个中间件返回错误，则中断链
    ///
    /// # 参数
    ///
    /// * `task_name` - 任务名称
    ///
    /// # 返回
    ///
    /// - `Ok(())` - 所有中间件执行成功
    /// - `Err(MiddlewareError)` - 某个中间件返回错误，链被中断
    pub async fn before(&self, task_name: &str) -> Result<(), MiddlewareError> {
        for middleware in &self.middlewares {
            if let Err(e) = middleware.before(task_name).await {
                warn!(
                    middleware_name = %middleware.name(),
                    task_name = %task_name,
                    error = %e,
                    "Middleware before failed, interrupting chain"
                );
                return Err(e);
            }
        }
        Ok(())
    }

    /// 执行所有中间件的 after 钩子
    ///
    /// 按注册顺序执行，即使某个中间件失败也继续执行其他中间件
    ///
    /// # 参数
    ///
    /// * `task_name` - 任务名称
    /// * `result` - 任务执行结果
    ///
    /// # 返回
    ///
    /// - `Ok(())` - 所有中间件执行成功
    /// - `Err(MiddlewareError)` - 某个中间件返回错误（但所有中间件都会执行）
    pub async fn after(
        &self,
        task_name: &str,
        result: &Result<(), Box<dyn std::error::Error + Send + Sync>>,
    ) -> Result<(), MiddlewareError> {
        let mut has_error = false;

        for middleware in &self.middlewares {
            if let Err(e) = middleware.after(task_name, result).await {
                warn!(
                    middleware_name = %middleware.name(),
                    task_name = %task_name,
                    error = %e,
                    "Middleware after failed"
                );
                has_error = true;
            }
        }

        if has_error {
            Err(MiddlewareError::ExecutionFailed {
                name: "middleware-chain".to_string(),
                reason: "One or more middlewares failed".to_string(),
            })
        } else {
            Ok(())
        }
    }

    /// 获取中间件数量
    pub fn middleware_count(&self) -> usize {
        self.middlewares.len()
    }
}

impl Default for MiddlewareChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestMiddleware {
        name: String,
    }

    impl Middleware for TestMiddleware {
        fn name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_middleware_chain_add() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(TestMiddleware {
            name: "test-middleware".to_string(),
        }));

        assert_eq!(chain.middleware_count(), 1);
    }

    #[tokio::test]
    async fn test_middleware_chain_before() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(TestMiddleware {
            name: "test-middleware".to_string(),
        }));

        let result = chain.before("task-1").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_middleware_chain_after() {
        let mut chain = MiddlewareChain::new();
        chain.add(Arc::new(TestMiddleware {
            name: "test-middleware".to_string(),
        }));

        let task_result: Result<(), Box<dyn std::error::Error + Send + Sync>> = Ok(());
        let result = chain.after("task-1", &task_result).await;
        assert!(result.is_ok());
    }
}
