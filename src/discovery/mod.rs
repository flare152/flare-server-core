//! 统一服务发现与负载均衡模块
//!
//! 提供统一的服务发现抽象，支持多种后端（etcd、consul、DNS、Service Mesh），
//! 完全兼容 tower 生态系统，支持命名空间、版本控制和自定义标签。

pub mod backend;
pub mod config;
pub mod discover;
pub mod examples;
pub mod factory;
pub mod instance;

pub use discover::{ServiceDiscover, ServiceDiscoverUpdater, ServiceClient, ChannelService};
pub use backend::DiscoveryBackend;
pub use config::{
    DiscoveryConfig, NamespaceConfig, VersionConfig, TagFilter, LoadBalanceStrategy, BackendType,
    HealthCheckConfig,
};
pub use factory::{DiscoveryFactory, ServiceRegistry};
pub use instance::{ServiceInstance, InstanceMetadata};

