//! 服务实例定义

use std::collections::HashMap;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

/// 服务实例
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceInstance {
    /// 服务类型（如 "signaling-online", "message-orchestrator"）
    pub service_type: String,
    
    /// 实例 ID（唯一标识）
    pub instance_id: String,
    
    /// 服务地址
    pub address: SocketAddr,
    
    /// 命名空间
    pub namespace: Option<String>,
    
    /// 版本
    pub version: Option<String>,
    
    /// 自定义标签（用于过滤和路由）
    pub tags: HashMap<String, String>,
    
    /// 元数据
    pub metadata: InstanceMetadata,
    
    /// 是否健康
    pub healthy: bool,
    
    /// 权重（用于负载均衡）
    pub weight: u32,
}

/// 实例元数据
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct InstanceMetadata {
    /// 区域（如 "us-east-1", "cn-beijing"）
    pub region: Option<String>,
    
    /// 可用区（如 "us-east-1a"）
    pub zone: Option<String>,
    
    /// 环境（如 "prod", "staging", "dev"）
    pub environment: Option<String>,
    
    /// 自定义元数据
    pub custom: HashMap<String, String>,
}

impl ServiceInstance {
    /// 创建新的服务实例
    pub fn new(
        service_type: impl Into<String>,
        instance_id: impl Into<String>,
        address: SocketAddr,
    ) -> Self {
        Self {
            service_type: service_type.into(),
            instance_id: instance_id.into(),
            address,
            namespace: None,
            version: None,
            tags: HashMap::new(),
            metadata: InstanceMetadata::default(),
            healthy: true,
            weight: 100,
        }
    }

    /// 设置命名空间
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    /// 设置版本
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// 添加标签
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// 设置权重
    pub fn with_weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }

    /// 设置健康状态
    pub fn with_health(mut self, healthy: bool) -> Self {
        self.healthy = healthy;
        self
    }

    /// 转换为 HTTP URL
    pub fn to_http_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// 转换为 gRPC URI
    pub fn to_grpc_uri(&self) -> String {
        format!("http://{}", self.address)
    }

    /// 检查是否匹配标签过滤器
    pub fn matches_tags(&self, filters: &HashMap<String, String>) -> bool {
        filters.iter().all(|(key, value)| {
            self.tags.get(key).map(|v| v == value).unwrap_or(false)
        })
    }

    /// 检查是否匹配版本
    pub fn matches_version(&self, version: Option<&str>) -> bool {
        match (version, &self.version) {
            (None, _) => true,
            (Some(v), Some(inst_v)) => v == inst_v,
            (Some(_), None) => false,
        }
    }

    /// 检查是否匹配命名空间
    pub fn matches_namespace(&self, namespace: Option<&str>) -> bool {
        match (namespace, &self.namespace) {
            (None, _) => true,
            (Some(ns), Some(inst_ns)) => ns == inst_ns,
            (Some(_), None) => false,
        }
    }
}

