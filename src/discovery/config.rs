//! 服务发现配置

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 服务发现配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// 后端类型：etcd, consul, dns, mesh
    pub backend: BackendType,
    
    /// 后端特定配置
    pub backend_config: HashMap<String, serde_json::Value>,
    
    /// 命名空间配置
    pub namespace: Option<NamespaceConfig>,
    
    /// 版本配置
    pub version: Option<VersionConfig>,
    
    /// 标签过滤器
    pub tag_filters: Vec<TagFilter>,
    
    /// 负载均衡策略
    pub load_balance: LoadBalanceStrategy,
    
    /// 健康检查配置
    pub health_check: Option<HealthCheckConfig>,
    
    /// 刷新间隔（秒）
    pub refresh_interval: Option<u64>,
}

/// 后端类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    Etcd,
    Consul,
    Dns,
    Mesh,
}

impl std::str::FromStr for BackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "etcd" => Ok(BackendType::Etcd),
            "consul" => Ok(BackendType::Consul),
            "dns" | "dns-srv" => Ok(BackendType::Dns),
            "mesh" | "xds" | "envoy" => Ok(BackendType::Mesh),
            _ => Err(format!("Unknown backend type: {}", s)),
        }
    }
}

/// 命名空间配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceConfig {
    /// 默认命名空间
    pub default: Option<String>,
    
    /// 命名空间分隔符
    pub separator: Option<String>,
}

impl Default for NamespaceConfig {
    fn default() -> Self {
        Self {
            default: None,
            separator: Some("/".to_string()),
        }
    }
}

/// 版本配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConfig {
    /// 默认版本
    pub default: Option<String>,
    
    /// 版本格式（semver, custom）
    pub format: Option<String>,
    
    /// 是否启用版本路由
    pub enable_routing: bool,
}

impl Default for VersionConfig {
    fn default() -> Self {
        Self {
            default: None,
            format: Some("semver".to_string()),
            enable_routing: true,
        }
    }
}

/// 标签过滤器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagFilter {
    /// 标签键
    pub key: String,
    
    /// 标签值（可选，如果为 None 则只检查键是否存在）
    pub value: Option<String>,
    
    /// 匹配模式（exact, prefix, regex）
    pub pattern: Option<String>,
}

/// 负载均衡策略
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// 轮询
    RoundRobin,
    /// 随机
    Random,
    /// 一致性哈希
    ConsistentHash,
    /// 最少连接
    LeastConnections,
    /// 加权轮询
    WeightedRoundRobin,
    /// 加权随机
    WeightedRandom,
}

impl Default for LoadBalanceStrategy {
    fn default() -> Self {
        LoadBalanceStrategy::ConsistentHash
    }
}

impl std::str::FromStr for LoadBalanceStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace("-", "_").as_str() {
            "round_robin" | "roundrobin" => Ok(LoadBalanceStrategy::RoundRobin),
            "random" => Ok(LoadBalanceStrategy::Random),
            "consistent_hash" | "consistenthash" => Ok(LoadBalanceStrategy::ConsistentHash),
            "least_connections" | "leastconnections" | "least_conn" => {
                Ok(LoadBalanceStrategy::LeastConnections)
            }
            "weighted_round_robin" | "weightedroundrobin" => {
                Ok(LoadBalanceStrategy::WeightedRoundRobin)
            }
            "weighted_random" | "weightedrandom" => Ok(LoadBalanceStrategy::WeightedRandom),
            _ => Err(format!("Unknown load balance strategy: {}", s)),
        }
    }
}

/// 健康检查配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    /// 健康检查间隔（秒）
    pub interval: u64,
    
    /// 超时时间（秒）
    pub timeout: u64,
    
    /// 失败阈值（连续失败多少次后标记为不健康）
    pub failure_threshold: u32,
    
    /// 成功阈值（连续成功多少次后标记为健康）
    pub success_threshold: u32,
    
    /// 健康检查路径（HTTP）
    pub path: Option<String>,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: 10,
            timeout: 5,
            failure_threshold: 3,
            success_threshold: 2,
            path: Some("/health".to_string()),
        }
    }
}

