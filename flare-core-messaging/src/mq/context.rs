//! MQ Context 传播工具
//!
//! 提供统一的 Context ↔ Headers 转换逻辑，用于 Kafka、NATS 等消息队列。
//! 基于 `utils::context` 模块的 `ctx_to_map` 和 `map_to_ctx` 实现。

use flare_core_base::context::Ctx;
use std::collections::HashMap;

/// 将 Ctx 编码为 MQ Headers (HashMap<String, String>)
///
/// 基于 `utils::ctx_to_map`，确保与 gRPC metadata 键名一致。
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::mq::context::ctx_to_mq_headers;
/// use flare_server_core::context::{Context, Ctx};
///
/// let ctx: Ctx = std::sync::Arc::new(
///     Context::with_request_id("req-1")
///         .with_tenant_id("t1")
///         .with_user_id("u1")
/// );
/// let headers = ctx_to_mq_headers(&ctx);
/// assert_eq!(headers.get("x-request-id"), Some(&"req-1".to_string()));
/// ```
pub fn ctx_to_mq_headers(ctx: &Ctx) -> HashMap<String, String> {
    flare_core_base::utils::ctx_to_map(ctx)
}

/// 从 MQ Headers (HashMap<String, String>) 解码为 Ctx
///
/// 基于 `utils::map_to_ctx`，缺失的 request_id 会生成新 UUID。
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::mq::context::mq_headers_to_ctx;
/// use std::collections::HashMap;
///
/// let mut headers = HashMap::new();
/// headers.insert("x-tenant-id".to_string(), "t1".to_string());
/// headers.insert("x-user-id".to_string(), "u1".to_string());
/// let ctx = mq_headers_to_ctx(&headers);
/// assert_eq!(ctx.tenant_id(), Some("t1"));
/// assert_eq!(ctx.user_id(), Some("u1"));
/// ```
pub fn mq_headers_to_ctx(headers: &HashMap<String, String>) -> Ctx {
    flare_core_base::utils::map_to_ctx(headers)
}

/// 将 Ctx 合并到现有的 Headers 中
///
/// 用于生产者发送消息时，将 Context 透传到消息头。
///
/// # 参数
/// - `headers`: 现有的消息头（会被修改）
/// - `ctx`: 要透传的上下文
///
/// # 示例
///
/// ```rust,no_run
/// use flare_server_core::mq::context::merge_ctx_to_headers;
/// use flare_server_core::context::{Context, Ctx};
/// use std::collections::HashMap;
///
/// let ctx: Ctx = std::sync::Arc::new(
///     Context::with_request_id("req-1").with_tenant_id("t1")
/// );
/// let mut headers = HashMap::new();
/// headers.insert("content-type".to_string(), "application/json".to_string());
/// merge_ctx_to_headers(&mut headers, &ctx);
/// assert_eq!(headers.get("x-tenant-id"), Some(&"t1".to_string()));
/// ```
pub fn merge_ctx_to_headers(headers: &mut HashMap<String, String>, ctx: &Ctx) {
    let ctx_headers = ctx_to_mq_headers(ctx);
    for (key, value) in ctx_headers {
        headers.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flare_core_base::context::{ActorContext, ActorType, Context};

    #[test]
    fn test_ctx_roundtrip() {
        let mut ctx = Context::with_request_id("req-123")
            .with_trace_id("trace-456")
            .with_tenant_id("tenant-1")
            .with_user_id("user-1")
            .with_device_id("device-1")
            .with_platform("web")
            .with_session_id("session-1");

        let actor = ActorContext::new("actor-1")
            .with_type(ActorType::User)
            .with_role("admin")
            .with_attribute("department", "engineering");
        ctx = ctx.with_actor(actor);

        let ctx: Ctx = std::sync::Arc::new(ctx);

        // Ctx -> Headers
        let headers = ctx_to_mq_headers(&ctx);
        assert_eq!(headers.get("x-request-id"), Some(&"req-123".to_string()));
        assert_eq!(headers.get("x-trace-id"), Some(&"trace-456".to_string()));
        assert_eq!(headers.get("x-tenant-id"), Some(&"tenant-1".to_string()));
        assert_eq!(headers.get("x-user-id"), Some(&"user-1".to_string()));
        assert_eq!(headers.get("x-device-id"), Some(&"device-1".to_string()));
        assert_eq!(headers.get("x-platform"), Some(&"web".to_string()));
        assert_eq!(headers.get("x-session-id"), Some(&"session-1".to_string()));
        assert_eq!(headers.get("x-actor-id"), Some(&"actor-1".to_string()));
        assert_eq!(headers.get("x-actor-type"), Some(&"1".to_string())); // ActorType::User = 1
        assert_eq!(headers.get("x-actor-roles"), Some(&"admin".to_string()));
        assert_eq!(
            headers.get("x-actor-attr-department"),
            Some(&"engineering".to_string())
        );

        // Headers -> Ctx
        let restored_ctx = mq_headers_to_ctx(&headers);
        assert_eq!(restored_ctx.request_id(), "req-123");
        assert_eq!(restored_ctx.trace_id(), "trace-456");
        assert_eq!(restored_ctx.tenant_id(), Some("tenant-1"));
        assert_eq!(restored_ctx.user_id(), Some("user-1"));
        assert_eq!(restored_ctx.device_id(), Some("device-1"));
        assert_eq!(restored_ctx.platform(), Some("web"));
        assert_eq!(restored_ctx.session_id(), Some("session-1"));

        let actor = restored_ctx.actor().unwrap();
        assert_eq!(actor.actor_id(), "actor-1");
        assert_eq!(actor.actor_type, ActorType::User);
        assert!(actor.roles().iter().any(|r| r.as_ref() == "admin"));
    }

    #[test]
    fn test_merge_ctx_to_headers() {
        let ctx: Ctx = std::sync::Arc::new(Context::with_request_id("req-1").with_tenant_id("t1"));

        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("x-custom".to_string(), "value".to_string());

        merge_ctx_to_headers(&mut headers, &ctx);

        // 原有 headers 保留
        assert_eq!(
            headers.get("content-type"),
            Some(&"application/json".to_string())
        );
        assert_eq!(headers.get("x-custom"), Some(&"value".to_string()));

        // Context headers 添加
        assert_eq!(headers.get("x-request-id"), Some(&"req-1".to_string()));
        assert_eq!(headers.get("x-tenant-id"), Some(&"t1".to_string()));
    }
}
