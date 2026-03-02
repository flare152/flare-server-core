//! 统一的 Metadata 编解码模块
//!
//! 提供高效的 Context <-> gRPC Metadata 双向转换，减少重复代码和性能开销。

use tonic::metadata::MetadataMap;
use crate::context::{
    Context, TenantContext, RequestContext, TraceContext, ActorContext, DeviceContext,
    ActorType, DevicePriority,
};

/// 将 Context 编码到 gRPC Metadata
///
/// 这是最高效的方式，一次性设置所有上下文信息。
pub fn encode_context_to_metadata(metadata: &mut MetadataMap, ctx: &Context) {
    // 1. 设置租户上下文
    if let Some(tenant) = ctx.tenant() {
        encode_tenant_to_metadata(metadata, tenant);
    } else if let Some(tenant_id) = ctx.tenant_id() {
        if !tenant_id.is_empty() {
            // 尝试解析为 MetadataValue
            match tenant_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                Ok(val) => {
                    metadata.insert("x-tenant-id", val);
                }
                Err(e) => {
                    // 如果解析失败，尝试使用 Binary 类型
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        error = %e,
                        "Failed to parse tenant_id as MetadataValue<Ascii>, trying Binary"
                    );
                    if let Ok(key) = "x-tenant-id".parse::<tonic::metadata::MetadataKey<tonic::metadata::Binary>>() {
                        let val = tonic::metadata::MetadataValue::<tonic::metadata::Binary>::from_bytes(tenant_id.as_bytes());
                        metadata.insert_bin(key, val);
                    } else {
                        tracing::error!(
                            tenant_id = %tenant_id,
                            "Failed to create MetadataKey<Binary> for tenant_id"
                        );
                    }
                }
            }
        } else {
            tracing::warn!("Context has empty tenant_id, skipping metadata encoding");
        }
    } else {
        tracing::debug!("Context has no tenant_id, skipping tenant metadata encoding");
    }

    // 2. 设置请求上下文
    if let Some(req_ctx) = ctx.request() {
        encode_request_context_to_metadata(metadata, req_ctx);
    } else {
        // 从 Context 字段构建 RequestContext
        let req_ctx: RequestContext = ctx.into();
        encode_request_context_to_metadata(metadata, &req_ctx);
    }

    // 3. 设置独立的上下文（如果存在）
    if let Some(trace) = ctx.trace() {
        encode_trace_to_metadata(metadata, trace);
    }
    if let Some(actor) = ctx.actor() {
        encode_actor_to_metadata(metadata, actor);
    }
    if let Some(device) = ctx.device() {
        encode_device_to_metadata(metadata, device);
    }
}

/// 从 gRPC Metadata 解码为 Context
///
/// 这是最高效的方式，一次性提取所有上下文信息。
pub fn decode_context_from_metadata(metadata: &MetadataMap) -> Option<Context> {
    // 提取 request_id（必需）
    let request_id = metadata
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let mut ctx = Context::with_request_id(request_id);

    // 提取租户上下文
    if let Some(tenant) = decode_tenant_from_metadata(metadata) {
        ctx = ctx.with_tenant(tenant);
    } else {
        // 如果没有完整的 TenantContext，尝试从 x-tenant-id 直接提取
        if let Some(tenant_id) = metadata
            .get("x-tenant-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
        {
            ctx = ctx.with_tenant_id(tenant_id);
        } else {
            // 调试：记录 tenant_id 缺失的情况
            tracing::debug!(
                request_id = %ctx.request_id(),
                "No tenant_id found in metadata, Context will have no tenant_id"
            );
        }
    }

    // 提取请求上下文
    if let Some(req_ctx) = decode_request_context_from_metadata(metadata) {
        ctx = ctx.with_request(req_ctx);
    }

    // 提取独立的上下文（如果 RequestContext 中没有）
    if ctx.trace().is_none() {
        if let Some(trace) = decode_trace_from_metadata(metadata) {
            ctx = ctx.with_trace(trace);
        }
    }
    if ctx.actor().is_none() {
        if let Some(actor) = decode_actor_from_metadata(metadata) {
            ctx = ctx.with_actor(actor);
        }
    }
    if ctx.device().is_none() {
        if let Some(device) = decode_device_from_metadata(metadata) {
            ctx = ctx.with_device(device);
        }
    }

    // 提取其他字段
    if let Some(trace_id) = metadata
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_trace_id(trace_id);
    }

    if let Some(user_id) = metadata
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_user_id(user_id);
    }

    if let Some(session_id) = metadata
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
    {
        ctx = ctx.with_session_id(session_id);
    }

    Some(ctx)
}

// ========== 租户上下文编解码 ==========

fn encode_tenant_to_metadata(metadata: &mut MetadataMap, tenant: &TenantContext) {
    if !tenant.tenant_id.is_empty() {
        if let Ok(val) = tenant.tenant_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-tenant-id", val);
        }
    }
    if !tenant.business_type.is_empty() {
        if let Ok(val) = tenant.business_type.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-business-type", val);
        }
    }
    if !tenant.environment.is_empty() {
        if let Ok(val) = tenant.environment.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-environment", val);
        }
    }
    if !tenant.organization_id.is_empty() {
        if let Ok(val) = tenant.organization_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-organization-id", val);
        }
    }
    for (key, value) in &tenant.labels {
        let header_name = format!("x-tenant-label-{}", key);
        if let Ok(key_val) = header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.insert(key_val, val);
            }
        }
    }
    for (key, value) in &tenant.attributes {
        let header_name = format!("x-tenant-attr-{}", key);
        if let Ok(key_val) = header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.insert(key_val, val);
            }
        }
    }
}

fn decode_tenant_from_metadata(metadata: &MetadataMap) -> Option<TenantContext> {
    let tenant_id = metadata
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let mut tenant = TenantContext::new(tenant_id);

    if let Some(val) = metadata.get("x-business-type").and_then(|v| v.to_str().ok()) {
        tenant.business_type = val.to_string();
    }
    if let Some(val) = metadata.get("x-environment").and_then(|v| v.to_str().ok()) {
        tenant.environment = val.to_string();
    }
    if let Some(val) = metadata.get("x-organization-id").and_then(|v| v.to_str().ok()) {
        tenant.organization_id = val.to_string();
    }

    // 提取标签和属性
    for entry in metadata.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Some(key_str) = key.as_str().strip_prefix("x-tenant-label-") {
                    if let Ok(value_str) = value.to_str() {
                        tenant.labels.insert(key_str.to_string(), value_str.to_string());
                    }
                } else if let Some(key_str) = key.as_str().strip_prefix("x-tenant-attr-") {
                    if let Ok(value_str) = value.to_str() {
                        tenant.attributes.insert(key_str.to_string(), value_str.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Some(tenant)
}

// ========== 请求上下文编解码 ==========

fn encode_request_context_to_metadata(metadata: &mut MetadataMap, req_ctx: &RequestContext) {
    if !req_ctx.request_id.is_empty() {
        if let Ok(val) = req_ctx.request_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-request-id", val);
        }
    }

    if let Some(trace) = &req_ctx.trace {
        encode_trace_to_metadata(metadata, trace);
    }

    if let Some(actor) = &req_ctx.actor {
        encode_actor_to_metadata(metadata, actor);
    }

    if let Some(device) = &req_ctx.device {
        encode_device_to_metadata(metadata, device);
    }

    if !req_ctx.channel.is_empty() {
        if let Ok(val) = req_ctx.channel.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-channel", val);
        }
    }

    if !req_ctx.user_agent.is_empty() {
        if let Ok(val) = req_ctx.user_agent.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-user-agent", val);
        }
    }

    for (key, value) in &req_ctx.attributes {
        let header_name = format!("x-request-attr-{}", key);
        if let Ok(key_val) = header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.insert(key_val, val);
            }
        }
    }
}

fn decode_request_context_from_metadata(metadata: &MetadataMap) -> Option<RequestContext> {
    let request_id = metadata
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let mut req_ctx = RequestContext::new(request_id);

    req_ctx.trace = decode_trace_from_metadata(metadata);
    req_ctx.actor = decode_actor_from_metadata(metadata);
    req_ctx.device = decode_device_from_metadata(metadata);

    if let Some(val) = metadata.get("x-channel").and_then(|v| v.to_str().ok()) {
        req_ctx.channel = val.to_string();
    }
    if let Some(val) = metadata.get("x-user-agent").and_then(|v| v.to_str().ok()) {
        req_ctx.user_agent = val.to_string();
    }

    // 提取 attributes
    for entry in metadata.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Some(key_str) = key.as_str().strip_prefix("x-request-attr-") {
                    if let Ok(value_str) = value.to_str() {
                        req_ctx.attributes.insert(key_str.to_string(), value_str.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Some(req_ctx)
}

// ========== 追踪上下文编解码 ==========

fn encode_trace_to_metadata(metadata: &mut MetadataMap, trace: &TraceContext) {
    if !trace.trace_id.is_empty() {
        if let Ok(val) = trace.trace_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-trace-id", val);
        }
    }
    if !trace.span_id.is_empty() {
        if let Ok(val) = trace.span_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-span-id", val);
        }
    }
    if !trace.parent_span_id.is_empty() {
        if let Ok(val) = trace.parent_span_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-parent-span-id", val);
        }
    }
    if !trace.sampled.is_empty() {
        if let Ok(val) = trace.sampled.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-trace-sampled", val);
        }
    }
    for (key, value) in &trace.tags {
        let header_name = format!("x-trace-tag-{}", key);
        if let Ok(key_val) = header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.insert(key_val, val);
            }
        }
    }
}

fn decode_trace_from_metadata(metadata: &MetadataMap) -> Option<TraceContext> {
    let trace_id = metadata
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let mut trace = TraceContext::new(trace_id);

    if let Some(val) = metadata.get("x-span-id").and_then(|v| v.to_str().ok()) {
        trace.span_id = val.to_string();
    }
    if let Some(val) = metadata.get("x-parent-span-id").and_then(|v| v.to_str().ok()) {
        trace.parent_span_id = val.to_string();
    }
    if let Some(val) = metadata.get("x-trace-sampled").and_then(|v| v.to_str().ok()) {
        trace.sampled = val.to_string();
    }

    // 提取 tags
    for entry in metadata.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Some(key_str) = key.as_str().strip_prefix("x-trace-tag-") {
                    if let Ok(value_str) = value.to_str() {
                        trace.tags.insert(key_str.to_string(), value_str.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Some(trace)
}

// ========== 操作者上下文编解码 ==========

fn encode_actor_to_metadata(metadata: &mut MetadataMap, actor: &ActorContext) {
    if !actor.actor_id.is_empty() {
        if let Ok(val) = actor.actor_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            if actor.actor_type == ActorType::User {
                metadata.insert("x-user-id", val.clone());
            }
            metadata.insert("x-actor-id", val);
        }
    }
    if actor.actor_type != ActorType::Unspecified {
        if let Ok(val) = format!("{}", actor.actor_type as i32).parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-actor-type", val);
        }
    }
    for role in &actor.roles {
        if !role.is_empty() {
            if let Ok(val) = role.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.append("x-actor-roles", val);
            }
        }
    }
    for (key, value) in &actor.attributes {
        let header_name = format!("x-actor-attr-{}", key);
        if let Ok(key_val) = header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.insert(key_val, val);
            }
        }
    }
}

fn decode_actor_from_metadata(metadata: &MetadataMap) -> Option<ActorContext> {
    let actor_id = metadata
        .get("x-actor-id")
        .or_else(|| metadata.get("x-user-id"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let actor_type = metadata
        .get("x-actor-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i32>().ok())
        .map(ActorType::from)
        .unwrap_or(ActorType::Unspecified);

    let mut actor = ActorContext::new(actor_id).with_type(actor_type);

    // 提取角色
    for val in metadata.get_all("x-actor-roles") {
        if let Ok(role) = val.to_str() {
            actor.roles.push(role.to_string());
        }
    }

    // 提取属性
    for entry in metadata.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Some(key_str) = key.as_str().strip_prefix("x-actor-attr-") {
                    if let Ok(value_str) = value.to_str() {
                        actor.attributes.insert(key_str.to_string(), value_str.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Some(actor)
}

// ========== 设备上下文编解码 ==========

fn encode_device_to_metadata(metadata: &mut MetadataMap, device: &DeviceContext) {
    if !device.device_id.is_empty() {
        if let Ok(val) = device.device_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-device-id", val);
        }
    }
    if !device.platform.is_empty() {
        if let Ok(val) = device.platform.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-device-platform", val);
        }
    }
    if !device.ip_address.is_empty() {
        if let Ok(val) = device.ip_address.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-client-ip", val);
        }
    }
    if device.priority != DevicePriority::Unspecified {
        if let Ok(val) = format!("{}", device.priority as i32).parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-device-priority", val);
        }
    }
    if device.token_version > 0 {
        if let Ok(val) = format!("{}", device.token_version).parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
            metadata.insert("x-device-token-version", val);
        }
    }
    for (key, value) in &device.attributes {
        let header_name = format!("x-device-attr-{}", key);
        if let Ok(key_val) = header_name.parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>() {
            if let Ok(val) = value.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                metadata.insert(key_val, val);
            }
        }
    }
}

fn decode_device_from_metadata(metadata: &MetadataMap) -> Option<DeviceContext> {
    let device_id = metadata
        .get("x-device-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())?;

    let mut device = DeviceContext::new(device_id);

    if let Some(val) = metadata.get("x-device-platform").and_then(|v| v.to_str().ok()) {
        device.platform = val.to_string();
    }
    if let Some(val) = metadata.get("x-client-ip").and_then(|v| v.to_str().ok()) {
        device.ip_address = val.to_string();
    }
    if let Some(val) = metadata.get("x-device-priority").and_then(|v| v.to_str().ok()) {
        if let Ok(priority) = val.parse::<i32>() {
            device.priority = DevicePriority::from(priority);
        }
    }
    if let Some(val) = metadata.get("x-device-token-version").and_then(|v| v.to_str().ok()) {
        if let Ok(version) = val.parse::<i64>() {
            device.token_version = version;
        }
    }

    // 提取属性
    for entry in metadata.iter() {
        match entry {
            tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                if let Some(key_str) = key.as_str().strip_prefix("x-device-attr-") {
                    if let Ok(value_str) = value.to_str() {
                        device.attributes.insert(key_str.to_string(), value_str.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    Some(device)
}

