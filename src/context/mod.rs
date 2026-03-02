//! 上下文模块
//!
//! 提供请求上下文、租户上下文等类型定义，以及核心 Context 系统。

use std::collections::HashMap;

// 核心类型定义
/// 租户上下文
///
/// 用于多租户隔离，包含租户的基本信息和扩展属性
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TenantContext {
    /// 租户ID（必需）
    pub tenant_id: String,
    /// 业务类型（如 "im", "customer_service", "ai_chat"）
    pub business_type: String,
    /// 环境（如 "production", "staging", "development"）
    pub environment: String,
    /// 组织ID
    pub organization_id: String,
    /// 标签（用于分类和过滤）
    pub labels: HashMap<String, String>,
    /// 扩展属性
    pub attributes: HashMap<String, String>,
}

impl TenantContext {
    /// 创建新的租户上下文
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            ..Default::default()
        }
    }

    /// 设置业务类型
    pub fn with_business_type(mut self, business_type: impl Into<String>) -> Self {
        self.business_type = business_type.into();
        self
    }

    /// 设置环境
    pub fn with_environment(mut self, environment: impl Into<String>) -> Self {
        self.environment = environment.into();
        self
    }

    /// 设置组织ID
    pub fn with_organization_id(mut self, organization_id: impl Into<String>) -> Self {
        self.organization_id = organization_id.into();
        self
    }

    /// 添加标签
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// 添加扩展属性
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// 操作者类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorType {
    Unspecified = 0,
    User = 1,
    Service = 2,
    TenantAdmin = 3,
    System = 4,
    Guest = 5,
}

impl Default for ActorType {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl From<i32> for ActorType {
    fn from(value: i32) -> Self {
        match value {
            1 => ActorType::User,
            2 => ActorType::Service,
            3 => ActorType::TenantAdmin,
            4 => ActorType::System,
            5 => ActorType::Guest,
            _ => ActorType::Unspecified,
        }
    }
}

impl From<ActorType> for i32 {
    fn from(value: ActorType) -> Self {
        value as i32
    }
}

/// 操作者上下文
///
/// 用于权限校验和审计
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActorContext {
    /// 操作者ID（必需）
    pub actor_id: String,
    /// 操作者类型
    pub actor_type: ActorType,
    /// 角色列表（RBAC）
    pub roles: Vec<String>,
    /// 扩展属性
    pub attributes: HashMap<String, String>,
}

impl ActorContext {
    /// 创建新的操作者上下文
    pub fn new(actor_id: impl Into<String>) -> Self {
        Self {
            actor_id: actor_id.into(),
            actor_type: ActorType::User,
            ..Default::default()
        }
    }

    /// 设置操作者类型
    pub fn with_type(mut self, actor_type: ActorType) -> Self {
        self.actor_type = actor_type;
        self
    }

    /// 添加角色
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }

    /// 添加扩展属性
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// 追踪上下文
///
/// 用于分布式追踪
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceContext {
    /// 追踪ID（全局唯一）
    pub trace_id: String,
    /// 当前Span ID
    pub span_id: String,
    /// 父Span ID（根Span为空）
    pub parent_span_id: String,
    /// 采样标志（"yes", "no", "force"）
    pub sampled: String,
    /// 追踪标签
    pub tags: HashMap<String, String>,
}

impl TraceContext {
    /// 创建新的追踪上下文
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            ..Default::default()
        }
    }

    /// 设置 span_id
    pub fn with_span_id(mut self, span_id: impl Into<String>) -> Self {
        self.span_id = span_id.into();
        self
    }

    /// 设置 parent_span_id
    pub fn with_parent_span_id(mut self, parent_span_id: impl Into<String>) -> Self {
        self.parent_span_id = parent_span_id.into();
        self
    }

    /// 添加标签
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }
}

/// 设备优先级枚举（用于多设备推送策略）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DevicePriority {
    Unspecified = 0,
    Low = 1,         // 低优先级（后台/离线）
    Normal = 2,      // 普通优先级（默认）
    High = 3,        // 高优先级（用户当前正在使用的设备）
    Critical = 4,    // 关键优先级（强制推送，忽略免打扰）
}

impl Default for DevicePriority {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl From<i32> for DevicePriority {
    fn from(value: i32) -> Self {
        match value {
            1 => DevicePriority::Low,
            2 => DevicePriority::Normal,
            3 => DevicePriority::High,
            4 => DevicePriority::Critical,
            _ => DevicePriority::Unspecified,
        }
    }
}

impl From<DevicePriority> for i32 {
    fn from(value: DevicePriority) -> Self {
        value as i32
    }
}

/// 连接质量指标（用于链接质量监控与智能路由）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectionQuality {
    /// 往返时延（毫秒）
    pub rtt_ms: i64,
    /// 丢包率（0.0-1.0）
    pub packet_loss_rate: f64,
    /// 最后测量时间戳（Unix 毫秒）
    pub last_measure_ts: i64,
    /// 网络类型（wifi/4g/5g/ethernet）
    pub network_type: String,
    /// 信号强度（dBm，移动网络）
    pub signal_strength: i32,
}

impl ConnectionQuality {
    /// 创建新的连接质量指标
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置往返时延
    pub fn with_rtt_ms(mut self, rtt_ms: i64) -> Self {
        self.rtt_ms = rtt_ms;
        self
    }

    /// 设置丢包率
    pub fn with_packet_loss_rate(mut self, rate: f64) -> Self {
        self.packet_loss_rate = rate;
        self
    }

    /// 设置网络类型
    pub fn with_network_type(mut self, network_type: impl Into<String>) -> Self {
        self.network_type = network_type.into();
        self
    }
}

/// 设备上下文
///
/// 用于设备管理和统计
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceContext {
    /// 设备ID（必需）
    pub device_id: String,
    /// 设备平台（如 "ios", "android", "web", "pc"）
    pub platform: String,
    /// 设备型号
    pub model: String,
    /// 操作系统版本
    pub os_version: String,
    /// 应用版本（语义化版本号）
    pub app_version: String,
    /// 语言环境（BCP 47格式）
    pub locale: String,
    /// 时区（IANA格式）
    pub timezone: String,
    /// IP地址（IPv4/IPv6）
    pub ip_address: String,
    /// 扩展属性
    pub attributes: HashMap<String, String>,
    /// 设备优先级（用于多设备推送策略）
    pub priority: DevicePriority,
    /// Token 版本（用于强制下线旧版本客户端）
    pub token_version: i64,
    /// 链接质量指标
    pub connection_quality: Option<ConnectionQuality>,
}

impl DeviceContext {
    /// 创建新的设备上下文
    pub fn new(device_id: impl Into<String>) -> Self {
        Self {
            device_id: device_id.into(),
            ..Default::default()
        }
    }

    /// 设置平台
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = platform.into();
        self
    }

    /// 设置IP地址
    pub fn with_ip_address(mut self, ip_address: impl Into<String>) -> Self {
        self.ip_address = ip_address.into();
        self
    }

    /// 添加扩展属性
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// 请求上下文
///
/// 包含请求的所有上下文信息
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestContext {
    /// 请求ID（必需，UUID格式）
    pub request_id: String,
    /// 追踪上下文（可选）
    pub trace: Option<TraceContext>,
    /// 操作者上下文（可选）
    pub actor: Option<ActorContext>,
    /// 设备上下文（可选）
    pub device: Option<DeviceContext>,
    /// 请求渠道（如 "websocket", "grpc", "http", "quic"）
    pub channel: String,
    /// 用户代理（HTTP User-Agent格式）
    pub user_agent: String,
    /// 扩展属性
    pub attributes: HashMap<String, String>,
}

impl RequestContext {
    /// 创建新的请求上下文
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            channel: "grpc".to_string(),
            ..Default::default()
        }
    }

    /// 设置追踪上下文
    pub fn with_trace(mut self, trace: TraceContext) -> Self {
        self.trace = Some(trace);
        self
    }

    /// 设置操作者上下文
    pub fn with_actor(mut self, actor: ActorContext) -> Self {
        self.actor = Some(actor);
        self
    }

    /// 设置设备上下文
    pub fn with_device(mut self, device: DeviceContext) -> Self {
        self.device = Some(device);
        self
    }

    /// 设置渠道
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = channel.into();
        self
    }

    /// 设置用户代理
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// 添加扩展属性
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// 获取用户ID（便捷方法）
    ///
    /// 如果 actor 是 User 类型，返回 actor_id
    pub fn user_id(&self) -> Option<&str> {
        self.actor
            .as_ref()
            .filter(|a| a.actor_type == ActorType::User)
            .map(|a| a.actor_id.as_str())
    }

    /// 获取操作者ID（便捷方法）
    pub fn actor_id(&self) -> Option<&str> {
        self.actor.as_ref().map(|a| a.actor_id.as_str())
    }
}

/// 审计上下文（用于合规和安全审计）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditContext {
    /// 操作者（必需）
    pub actor: ActorContext,
    /// 操作时间（Unix 毫秒时间戳）
    pub operated_at: i64,
    /// 操作原因
    pub reason: String,
    /// 扩展元数据
    pub metadata: HashMap<String, String>,
}

impl AuditContext {
    /// 创建新的审计上下文
    pub fn new(actor: ActorContext) -> Self {
        Self {
            actor,
            operated_at: chrono::Utc::now().timestamp_millis(),
            ..Default::default()
        }
    }

    /// 设置操作时间
    pub fn with_operated_at(mut self, timestamp: i64) -> Self {
        self.operated_at = timestamp;
        self
    }

    /// 设置操作原因
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// 添加元数据
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

// 核心 Context 系统
pub mod core;
// 类型映射表（用于自定义数据存储）
pub mod typemap;

// 类型转换（可选，需要启用 proto feature）
#[cfg(feature = "proto")]
pub mod conversions;

// 重新导出核心 Context 类型
pub use core::{Context, ContextExt, ContextError};
pub use typemap::TypeMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_context_builder() {
        let tenant = TenantContext::new("tenant-123")
            .with_business_type("im")
            .with_environment("production")
            .with_label("region", "us-east-1");

        assert_eq!(tenant.tenant_id, "tenant-123");
        assert_eq!(tenant.business_type, "im");
        assert_eq!(tenant.environment, "production");
        assert_eq!(tenant.labels.get("region"), Some(&"us-east-1".to_string()));
    }

    #[test]
    fn test_request_context_builder() {
        let ctx = RequestContext::new("req-123")
            .with_actor(ActorContext::new("user-456"))
            .with_trace(TraceContext::new("trace-789"));

        assert_eq!(ctx.request_id, "req-123");
        assert_eq!(ctx.actor.as_ref().unwrap().actor_id, "user-456");
        assert_eq!(ctx.trace.as_ref().unwrap().trace_id, "trace-789");
        assert_eq!(ctx.user_id(), Some("user-456"));
    }
}

