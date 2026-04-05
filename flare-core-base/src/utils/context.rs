//! Context 工具函数 (基础,不依赖 gRPC)

use crate::context::{ActorContext, ActorType, Context, Ctx, keys};
use std::collections::HashMap;

// -----------------------------------------------------------------------------
// Context 提取函数 (基础)
// -----------------------------------------------------------------------------

/// 从 Ctx 中提取租户ID（必需版本）
pub fn require_tenant_id_from_ctx(ctx: &Ctx) -> Result<String, &'static str> {
    ctx.tenant_id()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or("Tenant ID is required in context")
}

/// 从 Ctx 中提取用户ID（必需版本）
///
/// 优先从 actor 中获取,否则从 user_id 字段获取。
pub fn require_user_id_from_ctx(ctx: &Ctx) -> Result<String, &'static str> {
    if let Some(actor) = ctx.actor()
        && !actor.actor_id().is_empty()
    {
        return Ok(actor.actor_id().to_string());
    }
    ctx.user_id()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or("User ID is required in context")
}

/// 从 Ctx 中提取会话ID（可选版本）
pub fn extract_session_id_from_ctx(ctx: &Ctx) -> Option<String> {
    ctx.session_id().map(|s| s.to_string())
}

/// 从 Ctx 中提取请求ID（必需版本）
pub fn require_request_id_from_ctx(ctx: &Ctx) -> Result<String, &'static str> {
    let request_id = ctx.request_id();
    if request_id.is_empty() {
        return Err("Request ID is required in context");
    }
    Ok(request_id.to_string())
}

// -----------------------------------------------------------------------------
// Map <-> Ctx 转换函数
// -----------------------------------------------------------------------------

/// 将 Ctx 编码为 HashMap<String, String>
///
/// 用于消息队列、HTTP headers 等场景的上下文传递。
/// 键名与 gRPC metadata 一致,便于跨边界透传。
pub fn ctx_to_map(ctx: &Ctx) -> HashMap<String, String> {
    let mut map = HashMap::new();

    if !ctx.request_id().is_empty() {
        map.insert(keys::REQUEST_ID.to_string(), ctx.request_id().to_string());
    }
    if !ctx.trace_id().is_empty() {
        map.insert(keys::TRACE_ID.to_string(), ctx.trace_id().to_string());
    }
    if let Some(t) = ctx.tenant_id() {
        if !t.is_empty() {
            map.insert(keys::TENANT_ID.to_string(), t.to_string());
        }
    }
    if let Some(u) = ctx.user_id() {
        if !u.is_empty() {
            map.insert(keys::USER_ID.to_string(), u.to_string());
        }
    }
    if let Some(d) = ctx.device_id() {
        if !d.is_empty() {
            map.insert(keys::DEVICE_ID.to_string(), d.to_string());
        }
    }
    if let Some(p) = ctx.platform() {
        if !p.is_empty() {
            map.insert(keys::PLATFORM.to_string(), p.to_string());
        }
    }
    if let Some(s) = ctx.session_id() {
        if !s.is_empty() {
            map.insert(keys::SESSION_ID.to_string(), s.to_string());
        }
    }

    // Actor 信息
    if let Some(actor) = ctx.actor() {
        if !actor.actor_id().is_empty() {
            map.insert("x-actor-id".to_string(), actor.actor_id().to_string());
            if actor.actor_type == ActorType::User {
                map.insert(keys::USER_ID.to_string(), actor.actor_id().to_string());
            }
        }
        if actor.actor_type != ActorType::Unspecified {
            map.insert(
                "x-actor-type".to_string(),
                i32::from(actor.actor_type).to_string(),
            );
        }
        if !actor.roles().is_empty() {
            let roles: Vec<&str> = actor.roles().iter().map(|r| r.as_ref()).collect();
            map.insert("x-actor-roles".to_string(), roles.join(","));
        }
        for (k, v) in actor.attributes().iter() {
            map.insert(
                format!("x-actor-attr-{}", k.as_ref()),
                v.as_ref().to_string(),
            );
        }
    }

    map
}

/// 从 HashMap<String, String> 解码为 Ctx
///
/// 缺失的 request_id 会生成新 UUID。不设置 cancel/deadline,仅还原链路元数据。
pub fn map_to_ctx(map: &HashMap<String, String>) -> Ctx {
    let request_id = map
        .get(keys::REQUEST_ID)
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut ctx = Context::with_request_id(request_id);

    if let Some(v) = map.get(keys::TRACE_ID).filter(|s| !s.is_empty()) {
        ctx = ctx.with_trace_id(v.as_str());
    }
    if let Some(v) = map.get(keys::TENANT_ID).filter(|s| !s.is_empty()) {
        ctx = ctx.with_tenant_id(v.as_str());
    }
    if let Some(v) = map.get(keys::USER_ID).filter(|s| !s.is_empty()) {
        ctx = ctx.with_user_id(v.as_str());
    }
    if let Some(v) = map.get(keys::DEVICE_ID).filter(|s| !s.is_empty()) {
        ctx = ctx.with_device_id(v.as_str());
    }
    if let Some(v) = map.get(keys::PLATFORM).filter(|s| !s.is_empty()) {
        ctx = ctx.with_platform(v.as_str());
    }
    if let Some(v) = map.get(keys::SESSION_ID).filter(|s| !s.is_empty()) {
        ctx = ctx.with_session_id(v.as_str());
    }

    // Actor 信息
    if let Some(actor) = decode_actor_from_map(map) {
        ctx = ctx.with_actor(actor);
    }

    std::sync::Arc::new(ctx)
}

fn decode_actor_from_map(map: &HashMap<String, String>) -> Option<ActorContext> {
    let actor_id = map
        .get("x-actor-id")
        .or_else(|| map.get(keys::USER_ID))
        .filter(|s| !s.is_empty())?
        .clone();

    let actor_type = map
        .get("x-actor-type")
        .and_then(|s| s.parse::<i32>().ok())
        .map(ActorType::from)
        .unwrap_or(ActorType::Unspecified);

    let mut actor = ActorContext::new(actor_id).with_type(actor_type);

    if let Some(roles_str) = map.get("x-actor-roles").filter(|s| !s.is_empty()) {
        for role in roles_str.split(',') {
            let role = role.trim();
            if !role.is_empty() {
                actor = actor.with_role(role);
            }
        }
    }

    let prefix = "x-actor-attr-";
    for (key, value) in map.iter() {
        if let Some(attr_key) = key.strip_prefix(prefix) {
            if !value.is_empty() {
                actor = actor.with_attribute(attr_key, value.as_str());
            }
        }
    }

    Some(actor)
}
