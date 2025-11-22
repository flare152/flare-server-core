//! 统一服务发现使用示例
//!
//! 包含服务注册、发现、Channel 池、重试、健康检查和异步并发批量调用

/// 示例 0: 快速构建（服务注册 + 发现，推荐用于生产环境）
#[allow(dead_code)]
async fn example_quick_start() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryFactory, BackendType, ServiceInstance, InstanceMetadata,
    };
    use std::collections::HashMap;
    use std::net::SocketAddr;

    // 创建服务实例
    let instance = ServiceInstance {
        service_type: "signaling-online".to_string(),
        instance_id: format!("node-{}", uuid::Uuid::new_v4()),
        address: "127.0.0.1:8080".parse::<SocketAddr>()?,
        namespace: Some("production".to_string()),
        version: Some("v1.0.0".to_string()),
        tags: {
            let mut tags = HashMap::new();
            tags.insert("region".to_string(), "us-east-1".to_string());
            tags.insert("environment".to_string(), "production".to_string());
            tags
        },
        metadata: InstanceMetadata {
            region: Some("us-east-1".to_string()),
            zone: Some("us-east-1a".to_string()),
            environment: Some("production".to_string()),
            ..Default::default()
        },
        healthy: true,
        weight: 100,
    };

    // 快速构建：服务注册 + 发现（使用最优默认配置）
    let (registry, discover, _updater) = DiscoveryFactory::register_and_discover(
        BackendType::Etcd,
        vec!["http://localhost:2379".to_string()],
        "signaling-online".to_string(),
        instance,
    ).await?;

    // registry 会自动处理心跳续期
    // 当 registry 被 drop 时，会自动注销服务

    // 使用 discover 进行服务发现
    use crate::discovery::ServiceClient;
    let mut client = ServiceClient::new(discover);
    let _channel = client.get_channel().await?;

    // 创建 gRPC 客户端并调用
    // let mut grpc_client = YourServiceClient::new(channel);
    // let response = grpc_client.your_method(request).await?;

    // 保持 registry 存活（在实际应用中，应该保持到服务关闭）
    let _registry = registry;

    Ok(())
}

/// 示例 1: 服务注册
#[allow(dead_code)]
async fn example_service_registration() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, ServiceInstance, InstanceMetadata,
    };
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use serde_json::json;

    // 创建后端配置
    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    
    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: crate::discovery::LoadBalanceStrategy::RoundRobin,
        health_check: None,
        refresh_interval: Some(30),
    };

    // 创建后端
    let backend = DiscoveryFactory::create_backend(&config).await?;

    // 注册服务实例
    let instance = ServiceInstance {
        service_type: "signaling-online".to_string(),
        instance_id: "node-1".to_string(),
        address: "127.0.0.1:8080".parse::<SocketAddr>()?,
        namespace: Some("production".to_string()),
        version: Some("v1.0.0".to_string()),
        tags: {
            let mut tags = HashMap::new();
            tags.insert("region".to_string(), "us-east-1".to_string());
            tags.insert("environment".to_string(), "production".to_string());
            tags
        },
        metadata: InstanceMetadata {
            region: Some("us-east-1".to_string()),
            zone: Some("us-east-1a".to_string()),
            environment: Some("production".to_string()),
            ..Default::default()
        },
        healthy: true,
        weight: 100,
    };

    // 注册服务
    backend.register(instance.clone()).await?;
    println!("✅ Service registered: {}", instance.instance_id);

    // 注销服务
    backend.unregister(&instance.instance_id).await?;
    println!("✅ Service unregistered: {}", instance.instance_id);

    Ok(())
}

/// 示例 2: 使用 etcd 后端创建服务发现
#[allow(dead_code)]
async fn example_etcd_discovery() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
    };
    use std::collections::HashMap;
    use serde_json::json;

    // 配置 etcd 后端
    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    backend_config.insert("service_type".to_string(), json!("signaling-online"));
    backend_config.insert("ttl".to_string(), json!(30));

    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::ConsistentHash,
        health_check: None,
        refresh_interval: Some(30),
    };

    // 创建服务发现器（带自动刷新）
    let (discover, updater) = DiscoveryFactory::create_discover(config).await?;

    // 注意：由于 Endpoint 不是 tower::Service，不能直接用于 Balance
    // 实际使用时应该：
    // 1. 从 discover 获取实例列表
    // 2. 手动选择实例（根据负载均衡策略）
    // 3. 创建 Channel 并创建客户端
    let _discover = discover;
    
    // 保存 updater 以便后续手动添加/移除实例
    let _updater = updater;
    
    Ok(())
}

/// 示例 2: 使用 Consul 后端
#[allow(dead_code)]
async fn example_consul_discovery() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
    };
    use std::collections::HashMap;
    use serde_json::json;

    let mut backend_config = HashMap::new();
    backend_config.insert("url".to_string(), json!("http://localhost:8500"));
    backend_config.insert("service_type".to_string(), json!("message-orchestrator"));

    let config = DiscoveryConfig {
        backend: BackendType::Consul,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::RoundRobin,
        health_check: None,
        refresh_interval: Some(30),
    };

    let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
    
    // 注意：由于 Endpoint 不是 tower::Service，不能直接用于 Balance
    // 实际使用时应该手动选择实例并创建 Channel
    let _discover = discover;
    
    Ok(())
}

/// 示例 3: 使用 DNS 后端
#[allow(dead_code)]
async fn example_dns_discovery() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
    };
    use std::collections::HashMap;
    use serde_json::json;
    use std::net::SocketAddr;

    let mut backend_config = HashMap::new();
    backend_config.insert("domain".to_string(), json!("local"));
    backend_config.insert("addresses".to_string(), json!([
        "127.0.0.1:8080".parse::<SocketAddr>()?,
        "127.0.0.1:8081".parse::<SocketAddr>()?,
    ]));
    backend_config.insert("service_type".to_string(), json!("storage-reader"));

    let config = DiscoveryConfig {
        backend: BackendType::Dns,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::Random,
        health_check: None,
        refresh_interval: Some(60),
    };

    let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
    
    // 注意：由于 Endpoint 不是 tower::Service，不能直接用于 Balance
    // 实际使用时应该手动选择实例并创建 Channel
    let _discover = discover;
    
    Ok(())
}

/// 示例 4: 与 tonic 客户端集成（Channel 池 + 重试）
#[allow(dead_code)]
async fn example_tonic_integration() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
    };
    use std::collections::HashMap;
    use serde_json::json;

    // 创建服务发现器
    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    backend_config.insert("service_type".to_string(), json!("signaling-online"));

    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::ConsistentHash,
        health_check: Some(crate::discovery::HealthCheckConfig {
            interval: 10,  // 每 10 秒检查一次
            timeout: 5,    // 超时 5 秒
            failure_threshold: 3,  // 连续失败 3 次后标记为不健康
            success_threshold: 2,  // 连续成功 2 次后标记为健康
            path: Some("/health".to_string()),  // 健康检查路径
        }),
        refresh_interval: Some(30),
    };

    let (discover, updater) = DiscoveryFactory::create_discover(config).await?;
    
    // 创建服务客户端（集成 Channel 缓存和 P2C 负载均衡）
    use crate::discovery::ServiceClient;
    let mut client = ServiceClient::new(discover);
    
    // Channel 池：每个 Endpoint 对应一个缓存的 Channel，自动复用
    // 重试：使用 tower::retry 中间件
    // use tower::retry::RetryLayer;
    // use tower::ServiceBuilder;
    // use std::time::Duration;
    
    // 示例：进行多次 RPC 调用（带重试）
    for i in 0..5 {
        // 获取 Channel（自动负载均衡和缓存）
        let channel = client.get_channel().await
            .map_err(|e| format!("Failed to get channel: {}", e))?;
        
        // 创建 gRPC 客户端
        // let mut grpc_client = YourServiceClient::new(channel);
        
        // 使用重试策略
        // let retry_policy = tower::retry::RetryLayer::new(
        //     tower::retry::Retry::new(
        //         tower::retry::policy::ConstantBackoff::new(Duration::from_millis(100))
        //             .max_retries(3),
        //     ),
        // );
        // let mut client_with_retry = ServiceBuilder::new()
        //     .layer(retry_policy)
        //     .service(grpc_client);
        
        // let request = Request::new(YourRequest { ... });
        // let response = client_with_retry.call(request).await?;
        
        let _channel = channel; // 避免未使用变量警告
        let _i = i;
    }
    
    // 可以手动添加/移除实例（Channel 会自动创建和清理）
    // updater.insert("new-instance", channel_service).await;
    // updater.remove("old-instance").await;
    
    let _updater = updater;
    
    Ok(())
}

/// 示例 7: 健康检查（失败节点暂时剔除）
#[allow(dead_code)]
async fn example_health_check() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
    };
    use std::collections::HashMap;
    use serde_json::json;

    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    backend_config.insert("service_type".to_string(), json!("signaling-online"));

    // 启用健康检查
    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::RoundRobin,
        health_check: Some(crate::discovery::HealthCheckConfig {
            interval: 10,  // 每 10 秒检查一次
            timeout: 5,    // 超时 5 秒
            failure_threshold: 3,  // 连续失败 3 次后标记为不健康
            success_threshold: 2,  // 连续成功 2 次后标记为健康
            path: Some("/health".to_string()),  // 健康检查路径
        }),
        refresh_interval: Some(30),
    };

    let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
    
    // 健康检查会自动运行，失败的节点会被暂时剔除
    // 当节点恢复健康后，会自动重新加入负载均衡池
    
    use crate::discovery::ServiceClient;
    let mut client = ServiceClient::new(discover);
    
    // 获取 Channel（自动跳过不健康的节点）
    let channel = client.get_channel().await
        .map_err(|e| format!("Failed to get channel: {}", e))?;
    
    // 创建 gRPC 客户端并调用
    // let mut grpc_client = YourServiceClient::new(channel);
    // let response = grpc_client.your_method(request).await?;
    
    let _channel = channel;
    
    Ok(())
}

/// 示例 8: 异步并发批量调用
#[allow(dead_code)]
async fn example_batch_concurrent_calls() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
    };
    use std::collections::HashMap;
    use serde_json::json;
    use futures::future;

    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    backend_config.insert("service_type".to_string(), json!("signaling-online"));

    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::RoundRobin,
        health_check: None,
        refresh_interval: Some(30),
    };

    let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
    
    use crate::discovery::ServiceClient;
    let client = ServiceClient::new(discover);
    
    // 异步并发批量调用
    let requests: Vec<i32> = (0..10).collect();
    
    // 方式 1: 使用 futures::future::join_all 并发调用
    use std::sync::Arc;
    use tokio::sync::Mutex;
    
    let client = Arc::new(Mutex::new(client));
    let results: Vec<Result<(), Box<dyn std::error::Error + Send + Sync>>> = 
        future::join_all(requests.iter().map(|_| {
            let client = client.clone();
            async move {
                // 获取 Channel
                let _channel = {
                    let mut client = client.lock().await;
                    client.get_channel().await?
                };
                // 创建 gRPC 客户端并调用
                // let mut grpc_client = YourServiceClient::new(channel);
                // let response = grpc_client.your_method(request).await?;
                // Ok(response)
                Ok(())
            }
        })).await;
    
    // 处理结果
    for result in results {
        match result {
            Ok(_) => println!("✅ Request succeeded"),
            Err(e) => println!("❌ Request failed: {}", e),
        }
    }
    
    // 方式 2: 使用 tokio::spawn 并发调用
    let handles: Vec<_> = (0..10).map(|_i| {
        let client = client.clone();
        tokio::spawn(async move {
            // 获取 Channel
            let _channel = {
                let mut client = client.lock().await;
                client.get_channel().await?
            };
            // 创建 gRPC 客户端并调用
            // let mut grpc_client = YourServiceClient::new(channel);
            // let request = Request::new(YourRequest { id: i });
            // let response = grpc_client.your_method(request).await?;
            // Ok(response)
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        })
    }).collect();
    
    // 等待所有任务完成
    let results: Vec<_> = future::join_all(handles).await;
    for result in results {
        match result {
            Ok(Ok(_)) => println!("✅ Task succeeded"),
            Ok(Err(e)) => println!("❌ Task failed: {}", e),
            Err(e) => println!("❌ Task panicked: {}", e),
        }
    }
    
    Ok(())
}

/// 示例 5: 带命名空间和版本控制
#[allow(dead_code)]
async fn example_with_namespace_and_version() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy,
        NamespaceConfig, VersionConfig,
    };
    use std::collections::HashMap;
    use serde_json::json;

    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    backend_config.insert("service_type".to_string(), json!("session"));

    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: Some(NamespaceConfig {
            default: Some("production".to_string()),
            separator: Some("/".to_string()),
        }),
        version: Some(VersionConfig {
            default: Some("v1.0.0".to_string()),
            format: Some("semver".to_string()),
            enable_routing: true,
        }),
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::ConsistentHash,
        health_check: None,
        refresh_interval: Some(30),
    };

    let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
    
    // 注意：由于 Endpoint 不是 tower::Service，不能直接用于 Balance
    // 实际使用时应该手动选择实例并创建 Channel
    let _discover = discover;
    
    Ok(())
}

/// 示例 6: 带自定义标签过滤
#[allow(dead_code)]
async fn example_with_tags() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use crate::discovery::{
        DiscoveryConfig, DiscoveryFactory, BackendType, LoadBalanceStrategy, TagFilter,
    };
    use std::collections::HashMap;
    use serde_json::json;

    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(["http://localhost:2379"]));
    backend_config.insert("service_type".to_string(), json!("push"));

    let config = DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![
            TagFilter {
                key: "environment".to_string(),
                value: Some("production".to_string()),
                pattern: Some("exact".to_string()),
            },
            TagFilter {
                key: "region".to_string(),
                value: Some("us-east-1".to_string()),
                pattern: Some("exact".to_string()),
            },
        ],
        load_balance: LoadBalanceStrategy::WeightedRoundRobin,
        health_check: None,
        refresh_interval: Some(30),
    };

    let (discover, _updater) = DiscoveryFactory::create_discover(config).await?;
    
    // 注意：由于 Endpoint 不是 tower::Service，不能直接用于 Balance
    // 实际使用时应该手动选择实例并创建 Channel
    let _discover = discover;
    
    Ok(())
}

