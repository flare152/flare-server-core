//! 上下文类型转换
//!
//! 提供与 `flare-proto` 的转换支持（可选，需要启用 `proto` feature）
//!
//! # 使用示例
//!
//! ```rust
//! use flare_server_core::context::{TenantContext, convert_from_proto};
//!
//! // 从 flare-proto 的 TenantContext 转换
//! #[cfg(feature = "proto")]
//! {
//!     let proto_tenant = flare_proto::common::TenantContext {
//!         tenant_id: "tenant-123".to_string(),
//!         ..Default::default()
//!     };
//!     let tenant: TenantContext = proto_tenant.into();
//! }
//! ```

#[cfg(feature = "proto")]
use crate::context::{
    ActorContext, ActorType, DeviceContext, DevicePriority, ConnectionQuality,
    RequestContext, TenantContext, TraceContext,
};

#[cfg(feature = "proto")]
impl From<flare_proto::common::TenantContext> for TenantContext {
    fn from(proto: flare_proto::common::TenantContext) -> Self {
        Self {
            tenant_id: proto.tenant_id,
            business_type: proto.business_type,
            environment: proto.environment,
            organization_id: proto.organization_id,
            labels: proto.labels,
            attributes: proto.attributes,
        }
    }
}

#[cfg(feature = "proto")]
impl From<TenantContext> for flare_proto::common::TenantContext {
    fn from(ctx: TenantContext) -> Self {
        Self {
            tenant_id: ctx.tenant_id,
            business_type: ctx.business_type,
            environment: ctx.environment,
            organization_id: ctx.organization_id,
            labels: ctx.labels,
            attributes: ctx.attributes,
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::ActorType> for ActorType {
    fn from(proto: flare_proto::common::ActorType) -> Self {
        match proto {
            flare_proto::common::ActorType::Unspecified => ActorType::Unspecified,
            flare_proto::common::ActorType::User => ActorType::User,
            flare_proto::common::ActorType::Service => ActorType::Service,
            flare_proto::common::ActorType::TenantAdmin => ActorType::TenantAdmin,
            flare_proto::common::ActorType::System => ActorType::System,
            flare_proto::common::ActorType::Guest => ActorType::Guest,
        }
    }
}

#[cfg(feature = "proto")]
impl From<ActorType> for flare_proto::common::ActorType {
    fn from(ty: ActorType) -> Self {
        match ty {
            ActorType::Unspecified => flare_proto::common::ActorType::Unspecified,
            ActorType::User => flare_proto::common::ActorType::User,
            ActorType::Service => flare_proto::common::ActorType::Service,
            ActorType::TenantAdmin => flare_proto::common::ActorType::TenantAdmin,
            ActorType::System => flare_proto::common::ActorType::System,
            ActorType::Guest => flare_proto::common::ActorType::Guest,
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::DevicePriority> for DevicePriority {
    fn from(proto: flare_proto::common::DevicePriority) -> Self {
        DevicePriority::from(proto as i32)
    }
}

#[cfg(feature = "proto")]
impl From<DevicePriority> for flare_proto::common::DevicePriority {
    fn from(priority: DevicePriority) -> Self {
        match priority {
            DevicePriority::Unspecified => flare_proto::common::DevicePriority::Unspecified,
            DevicePriority::Low => flare_proto::common::DevicePriority::Low,
            DevicePriority::Normal => flare_proto::common::DevicePriority::Normal,
            DevicePriority::High => flare_proto::common::DevicePriority::High,
            DevicePriority::Critical => flare_proto::common::DevicePriority::Critical,
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::ConnectionQuality> for ConnectionQuality {
    fn from(proto: flare_proto::common::ConnectionQuality) -> Self {
        Self {
            rtt_ms: proto.rtt_ms,
            packet_loss_rate: proto.packet_loss_rate,
            last_measure_ts: proto.last_measure_ts,
            network_type: proto.network_type,
            signal_strength: proto.signal_strength,
        }
    }
}

#[cfg(feature = "proto")]
impl From<ConnectionQuality> for flare_proto::common::ConnectionQuality {
    fn from(quality: ConnectionQuality) -> Self {
        Self {
            rtt_ms: quality.rtt_ms,
            packet_loss_rate: quality.packet_loss_rate,
            last_measure_ts: quality.last_measure_ts,
            network_type: quality.network_type,
            signal_strength: quality.signal_strength,
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::ActorContext> for ActorContext {
    fn from(proto: flare_proto::common::ActorContext) -> Self {
        Self {
            actor_id: proto.actor_id,
            actor_type: ActorType::from(proto.r#type),
            roles: proto.roles,
            attributes: proto.attributes,
        }
    }
}

#[cfg(feature = "proto")]
impl From<ActorContext> for flare_proto::common::ActorContext {
    fn from(ctx: ActorContext) -> Self {
        Self {
            actor_id: ctx.actor_id,
            r#type: ctx.actor_type as i32,
            roles: ctx.roles,
            attributes: ctx.attributes,
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::DeviceContext> for DeviceContext {
    fn from(proto: flare_proto::common::DeviceContext) -> Self {
        let priority = DevicePriority::from(proto.priority() as i32);
        let connection_quality = proto.connection_quality.map(Into::into);
        Self {
            device_id: proto.device_id,
            platform: proto.platform,
            model: proto.model,
            os_version: proto.os_version,
            app_version: proto.app_version,
            locale: proto.locale,
            timezone: proto.timezone,
            ip_address: proto.ip_address,
            attributes: proto.attributes,
            priority,
            token_version: proto.token_version,
            connection_quality,
        }
    }
}

#[cfg(feature = "proto")]
impl From<DeviceContext> for flare_proto::common::DeviceContext {
    fn from(ctx: DeviceContext) -> Self {
        Self {
            device_id: ctx.device_id,
            platform: ctx.platform,
            model: ctx.model,
            os_version: ctx.os_version,
            app_version: ctx.app_version,
            locale: ctx.locale,
            timezone: ctx.timezone,
            ip_address: ctx.ip_address,
            attributes: ctx.attributes,
            priority: ctx.priority as i32,
            token_version: ctx.token_version,
            connection_quality: ctx.connection_quality.map(Into::into),
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::TraceContext> for TraceContext {
    fn from(proto: flare_proto::common::TraceContext) -> Self {
        Self {
            trace_id: proto.trace_id,
            span_id: proto.span_id,
            parent_span_id: proto.parent_span_id,
            sampled: proto.sampled,
            tags: proto.tags,
        }
    }
}

#[cfg(feature = "proto")]
impl From<TraceContext> for flare_proto::common::TraceContext {
    fn from(ctx: TraceContext) -> Self {
        Self {
            trace_id: ctx.trace_id,
            span_id: ctx.span_id,
            parent_span_id: ctx.parent_span_id,
            sampled: ctx.sampled,
            tags: ctx.tags,
        }
    }
}

#[cfg(feature = "proto")]
impl From<flare_proto::common::RequestContext> for RequestContext {
    fn from(proto: flare_proto::common::RequestContext) -> Self {
        Self {
            request_id: proto.request_id,
            trace: proto.trace.map(Into::into),
            actor: proto.actor.map(Into::into),
            device: proto.device.map(Into::into),
            channel: proto.channel,
            user_agent: proto.user_agent,
            attributes: proto.attributes,
        }
    }
}

#[cfg(feature = "proto")]
impl From<RequestContext> for flare_proto::common::RequestContext {
    fn from(ctx: RequestContext) -> Self {
        Self {
            request_id: ctx.request_id,
            trace: ctx.trace.map(Into::into),
            actor: ctx.actor.map(Into::into),
            device: ctx.device.map(Into::into),
            channel: ctx.channel,
            user_agent: ctx.user_agent,
            attributes: ctx.attributes,
        }
    }
}

