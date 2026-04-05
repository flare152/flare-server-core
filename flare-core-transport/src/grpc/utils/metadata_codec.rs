//! 统一的 Metadata 编解码模块
//!
//! 提供高效的 Context <-> gRPC Metadata 双向转换，与 context/core 对齐。

use std::sync::Arc;
use tonic::metadata::MetadataMap;

use flare_core_base::context::{ActorContext, ActorType, Context, Ctx, keys};

/// 将 Context 编码到 gRPC Metadata
pub fn encode_context_to_metadata(metadata: &mut MetadataMap, ctx: &Context) {
    if !ctx.request_id().is_empty() {
        if let Ok(val) = ctx
            .request_id()
            .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
        {
            metadata.insert(keys::REQUEST_ID, val);
        }
    }
    if !ctx.trace_id().is_empty() {
        if let Ok(val) = ctx
            .trace_id()
            .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
        {
            metadata.insert(keys::TRACE_ID, val);
        }
    }
    if let Some(tenant_id) = ctx.tenant_id() {
        if !tenant_id.is_empty() {
            if let Ok(val) =
                tenant_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.insert(keys::TENANT_ID, val);
            }
        }
    }
    if let Some(user_id) = ctx.user_id() {
        if !user_id.is_empty() {
            if let Ok(val) =
                user_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.insert(keys::USER_ID, val);
            }
        }
    }
    if let Some(device_id) = ctx.device_id() {
        if !device_id.is_empty() {
            if let Ok(val) =
                device_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.insert(keys::DEVICE_ID, val);
            }
        }
    }
    if let Some(platform) = ctx.platform() {
        if !platform.is_empty() {
            if let Ok(val) =
                platform.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.insert(keys::PLATFORM, val);
            }
        }
    }
    if let Some(session_id) = ctx.session_id() {
        if !session_id.is_empty() {
            if let Ok(val) =
                session_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.insert(keys::SESSION_ID, val);
            }
        }
    }
    if let Some(actor) = ctx.actor() {
        encode_actor_to_metadata(metadata, actor);
    }
}

/// 从 gRPC Metadata 解码为 Context
pub fn decode_context_from_metadata(metadata: &MetadataMap) -> Option<Ctx> {
    let request_id = metadata
        .get(keys::REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut ctx = Context::with_request_id(request_id);

    if let Some(tenant_id) = metadata
        .get(keys::TENANT_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_tenant_id(tenant_id);
    }
    if let Some(trace_id) = metadata
        .get(keys::TRACE_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_trace_id(trace_id);
    }
    if let Some(user_id) = metadata
        .get(keys::USER_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_user_id(user_id);
    }
    if let Some(device_id) = metadata
        .get(keys::DEVICE_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_device_id(device_id);
    }
    if let Some(platform) = metadata
        .get(keys::PLATFORM)
        .or_else(|| metadata.get("x-device-platform"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_platform(platform);
    }
    if let Some(session_id) = metadata
        .get(keys::SESSION_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_session_id(session_id);
    }
    if let Some(actor) = decode_actor_from_metadata(metadata) {
        ctx = ctx.with_actor(actor);
    }

    Some(Arc::new(ctx))
}

fn encode_actor_to_metadata(metadata: &mut MetadataMap, actor: &ActorContext) {
    if !actor.actor_id().is_empty() {
        if let Ok(val) = actor
            .actor_id()
            .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
        {
            metadata.insert(keys::ACTOR_ID, val.clone());
            if actor.actor_type == ActorType::User {
                metadata.insert(keys::USER_ID, val);
            }
        }
    }
    if actor.actor_type != ActorType::Unspecified {
        if let Ok(val) = format!("{}", i32::from(actor.actor_type))
            .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
        {
            metadata.insert(keys::ACTOR_TYPE, val);
        }
    }
    for role in actor.roles().iter() {
        if !role.is_empty() {
            if let Ok(val) = role
                .as_ref()
                .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.append("x-actor-roles", val);
            }
        }
    }
    for (key, value) in actor.attributes().iter() {
        let header_name = format!("x-actor-attr-{}", key.as_ref());
        if let Ok(key_val) =
            header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>()
        {
            if let Ok(val) = value
                .as_ref()
                .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.insert(key_val, val);
            }
        }
    }
}

fn decode_actor_from_metadata(metadata: &MetadataMap) -> Option<ActorContext> {
    let actor_id = metadata
        .get(keys::ACTOR_ID)
        .or_else(|| metadata.get(keys::USER_ID))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let actor_type = metadata
        .get(keys::ACTOR_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i32>().ok())
        .map(ActorType::from)
        .unwrap_or(ActorType::Unspecified);

    let mut actor = ActorContext::new(actor_id).with_type(actor_type);

    for val in metadata.get_all("x-actor-roles") {
        if let Ok(role) = val.to_str() {
            if !role.is_empty() {
                actor = actor.with_role(role);
            }
        }
    }

    for entry in metadata.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Some(key_str) = key.as_str().strip_prefix("x-actor-attr-") {
                    if let Ok(value_str) = value.to_str() {
                        actor = actor.with_attribute(key_str, value_str);
                    }
                }
            }
            _ => {}
        }
    }

    Some(actor)
}
