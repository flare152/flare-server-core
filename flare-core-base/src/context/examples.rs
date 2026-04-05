//! Context 使用示例
//!
//! 展示如何在 IM / SDK / 后端服务中使用 Context 系统。

use std::time::Duration;
use tokio::time::sleep;
use crate::context::{Context, ContextExt};
use anyhow::Result;

/// 示例 1: 基本的 Context 使用
///
/// 展示如何创建 Context、设置超时、检查取消状态。
pub async fn example_basic_usage() -> Result<()> {
    // 创建根 Context
    let root_ctx = Context::root()
        .with_request_id("req-001")
        .with_trace_id("trace-001")
        .with_user_id("user-001")
        .with_tenant_id("tenant-001");

    println!("Request ID: {}", root_ctx.request_id());
    println!("Trace ID: {}", root_ctx.trace_id());
    println!("User ID: {:?}", root_ctx.user_id());
    println!("Tenant ID: {:?}", root_ctx.tenant_id());

    // 创建带超时的子 Context
    let timeout_ctx = root_ctx.with_timeout(Duration::from_secs(5));
    
    // 检查是否已取消
    if timeout_ctx.is_cancelled() {
        return Err(anyhow::anyhow!("Context already cancelled"));
    }

    // 获取剩余时间
    if let Some(remaining) = timeout_ctx.remaining_time() {
        println!("Remaining time: {:?}", remaining);
    }

    Ok(())
}

/// 示例 2: 在异步函数中传递 Context
///
/// 展示如何在异步调用链中传递 Context。
pub async fn example_async_chain() -> Result<()> {
    let ctx = Context::root()
        .with_request_id("req-002")
        .with_timeout(Duration::from_secs(10));

    // 在调用链中传递 Context
    let result = process_message(&ctx, "Hello, World!").await?;
    println!("Result: {}", result);

    Ok(())
}

/// 处理消息（模拟 IM 消息处理）
async fn process_message(ctx: &Context, content: &str) -> Result<String> {
    // 检查取消状态
    ctx.ensure_not_cancelled()?;

    // 调用下游服务
    let result = send_to_storage(ctx, content).await?;
    
    // 再次检查取消状态
    ctx.check_cancelled()?;

    Ok(result)
}

/// 发送到存储服务
async fn send_to_storage(ctx: &Context, content: &str) -> Result<String> {
    // 创建子 Context（继承父 Context 的取消和超时）
    let storage_ctx = ctx.child();

    // 模拟存储操作
    tokio::select! {
        result = async {
            sleep(Duration::from_millis(100)).await;
            Ok(format!("stored: {}", content))
        } => result,
        _ = storage_ctx.cancelled() => {
            Err(anyhow::anyhow!("storage operation cancelled"))
        }
    }
}

/// 示例 3: 取消传播
///
/// 展示父 Context 取消时，子 Context 自动取消。
pub async fn example_cancel_propagation() -> Result<()> {
    let parent = Context::root().with_request_id("req-003");
    let child1 = parent.child();
    let child2 = parent.child();

    // 启动子任务
    let task1 = tokio::spawn(async move {
        loop {
            if child1.is_cancelled() {
                println!("Child 1 cancelled");
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    });

    let task2 = tokio::spawn(async move {
        loop {
            if child2.is_cancelled() {
                println!("Child 2 cancelled");
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    });

    // 等待一小段时间
    sleep(Duration::from_millis(50)).await;

    // 取消父 Context
    println!("Cancelling parent context...");
    parent.cancel();

    // 等待子任务完成
    let _ = tokio::try_join!(task1, task2)?;

    Ok(())
}

/// 示例 4: 超时处理
///
/// 展示如何使用超时 Context。
pub async fn example_timeout() -> Result<()> {
    let ctx = Context::root()
        .with_request_id("req-004")
        .with_timeout(Duration::from_millis(100));

    // 模拟长时间运行的任务
    let result = tokio::select! {
        result = async {
            sleep(Duration::from_secs(1)).await;
            Ok("task completed")
        } => result,
        _ = ctx.cancelled() => {
            Err(anyhow::anyhow!("task timeout"))
        }
    };

    match result {
        Ok(_) => println!("Task completed"),
        Err(e) => println!("Task failed: {}", e),
    }

    Ok(())
}

/// 示例 5: IM SDK 风格的消息发送
///
/// 展示在 IM SDK 中如何使用 Context。
pub struct MessageService;

impl MessageService {
    /// 发送消息
    pub async fn send_message(
        &self,
        ctx: &Context,
        conversation_id: &str,
        content: &str,
    ) -> Result<String> {
        // 检查取消状态
        ctx.ensure_not_cancelled()?;

        // 验证参数
        if conversation_id.is_empty() {
            return Err(anyhow::anyhow!("conversation_id is required"));
        }

        // 创建子 Context 用于路由
        let route_ctx = ctx.child().with_session_id(conversation_id);

        // 路由消息
        let gateway = self.route_message(&route_ctx, conversation_id).await?;

        // 发送到网关
        let message_id = self.send_to_gateway(&route_ctx, &gateway, content).await?;

        Ok(message_id)
    }

    /// 路由消息
    async fn route_message(&self, ctx: &Context, conversation_id: &str) -> Result<String> {
        ctx.check_cancelled()?;

        // 模拟路由查找
        tokio::select! {
            result = async {
                sleep(Duration::from_millis(50)).await;
                Ok(format!("gateway-{}", conversation_id))
            } => result,
            _ = ctx.cancelled() => {
                Err(anyhow::anyhow!("routing cancelled"))
            }
        }
    }

    /// 发送到网关
    async fn send_to_gateway(
        &self,
        ctx: &Context,
        gateway: &str,
        content: &str,
    ) -> Result<String> {
        ctx.check_cancelled()?;

        // 模拟网络请求
        tokio::select! {
            result = async {
                sleep(Duration::from_millis(100)).await;
                Ok(format!("msg-{}", uuid::Uuid::new_v4()))
            } => result,
            _ = ctx.cancelled() => {
                Err(anyhow::anyhow!("send cancelled"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_usage() {
        example_basic_usage().await.unwrap();
    }

    #[tokio::test]
    async fn test_async_chain() {
        example_async_chain().await.unwrap();
    }

    #[tokio::test]
    async fn test_cancel_propagation() {
        example_cancel_propagation().await.unwrap();
    }

    #[tokio::test]
    async fn test_timeout() {
        example_timeout().await.unwrap();
    }

    #[tokio::test]
    async fn test_im_sdk_style() {
        let ctx = Context::root()
            .with_request_id("req-005")
            .with_user_id("user-001")
            .with_timeout(Duration::from_secs(5));

        let service = MessageService;
        let result = service
            .send_message(&ctx, "conv-001", "Hello, World!")
            .await;

        assert!(result.is_ok());
        println!("Message ID: {}", result.unwrap());
    }
}


