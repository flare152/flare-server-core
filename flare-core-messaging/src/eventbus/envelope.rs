//! 统一事件信封（分布式事件总线标准载荷）
//!
//! 用于跨服务、基于 Topic 的事件总线（Kafka / NATS 等）：按 `partition_key` 分区保证同流有序，
//! 适用于 IM（conversation_id）、订单（order_id）、审计（tenant_id）等任意业务流。

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

use flare_core_base::error::{FlareError, Result};

/// 统一事件信封：事件总线上的标准载荷（与具体业务解耦）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// 全局唯一事件 ID
    pub event_id: String,
    /// 事件类型（业务自定义，如 "message.created"、"order.shipped"）
    pub event_type: String,
    /// 分区键：用于 Kafka 等按 key 分区与过滤，同 key 内有序（IM 可为 conversation_id，订单可为 order_id）
    pub partition_key: String,
    /// 流内序号：用于同分区内的顺序与去重
    pub seq: u64,
    /// 业务载荷（protobuf / JSON 等，由业务解析）
    pub payload: Vec<u8>,
    /// 可选：事件发生时间戳（毫秒）
    #[serde(default)]
    pub timestamp_ms: Option<u64>,
    /// 可选：来源服务标识
    #[serde(default)]
    pub source: Option<String>,
}

impl EventEnvelope {
    /// 构造信封。`partition_key` 由业务定义（如会话 ID、订单 ID、租户 ID）。
    pub fn new(
        event_type: impl Into<String>,
        partition_key: impl Into<String>,
        seq: u64,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            event_type: event_type.into(),
            partition_key: partition_key.into(),
            seq,
            payload,
            timestamp_ms: None,
            source: None,
        }
    }

    /// 设置时间戳（毫秒）
    pub fn with_timestamp_ms(mut self, ms: u64) -> Self {
        self.timestamp_ms = Some(ms);
        self
    }

    /// 设置来源服务
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// 分区键引用（Kafka 等后端用于分区与顺序）
    pub fn partition_key(&self) -> &str {
        &self.partition_key
    }

    /// JSON 编码整封信封（历史兼容；不建议用于 MQ 热路径）。
    #[inline]
    pub fn to_json_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| FlareError::serialization_error(e.to_string()))
    }

    /// protobuf 编码整封信封（推荐：低开销、体积小）。
    ///
    /// 需要启用 `flare-server-core` 的 `proto` feature。
    #[cfg(feature = "proto")]
    #[inline]
    pub fn to_proto_bytes(&self) -> Result<Vec<u8>> {
        use prost::Message as _;
        let msg = flare_proto::common::EventBusEnvelope {
            event_id: self.event_id.clone(),
            event_type: self.event_type.clone(),
            partition_key: self.partition_key.clone(),
            seq: self.seq,
            payload: self.payload.clone(),
            timestamp_ms: self.timestamp_ms.unwrap_or(0),
            source: self.source.clone().unwrap_or_default(),
        };
        Ok(msg.encode_to_vec())
    }

    /// 从 protobuf bytes 解码信封。
    ///
    /// 需要启用 `flare-server-core` 的 `proto` feature。
    #[cfg(feature = "proto")]
    #[inline]
    pub fn from_proto_bytes(bytes: &[u8]) -> Result<Self> {
        use prost::Message as _;
        let msg = flare_proto::common::EventBusEnvelope::decode(bytes)
            .map_err(|e| FlareError::deserialization_error(e.to_string()))?;
        Ok(Self {
            event_id: msg.event_id,
            event_type: msg.event_type,
            partition_key: msg.partition_key,
            seq: msg.seq,
            payload: msg.payload,
            timestamp_ms: if msg.timestamp_ms == 0 {
                None
            } else {
                Some(msg.timestamp_ms)
            },
            source: if msg.source.is_empty() {
                None
            } else {
                Some(msg.source)
            },
        })
    }

    /// 可选约定：event_type 以 "operation." 开头时视为操作类（可由业务自行约定或忽略）
    pub fn is_operation(&self) -> bool {
        self.event_type.starts_with("operation.")
    }
}

impl fmt::Display for EventEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EventEnvelope({} type={} key={} seq={})",
            self.event_id, self.event_type, self.partition_key, self.seq
        )
    }
}
