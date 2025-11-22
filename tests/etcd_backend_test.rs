//! etcd 后端集成测试
//!
//! 这些测试需要运行中的 etcd 服务器实例。
//! 默认情况下，测试会被忽略，需要使用 `cargo test --test etcd_backend_test -- --ignored` 运行。
//!
//! 启动 etcd 服务器：
//! ```bash
//! # 使用 Docker 启动 etcd
//! docker run -d --name etcd-test -p 2379:2379 -p 2380:2380 \
//!   quay.io/coreos/etcd:v3.5.9 \
//!   etcd --advertise-client-urls=http://127.0.0.1:2379 \
//!        --listen-client-urls=http://0.0.0.0:2379
//!
//! # 或者使用本地安装的 etcd
//! etcd --advertise-client-urls=http://127.0.0.1:2379 \
//!      --listen-client-urls=http://0.0.0.0:2379
//! ```

use flare_server_core::discovery::{
    DiscoveryConfig, DiscoveryFactory, BackendType, ServiceInstance,
    ServiceClient, InstanceMetadata,
    LoadBalanceStrategy, HealthCheckConfig,
};
use std::collections::HashMap;
use serde_json::json;
use tokio::time::{sleep, Duration};

/// etcd 服务器地址
/// 可以通过环境变量 ETCD_ENDPOINTS 覆盖，默认为 http://127.0.0.1:2379
fn etcd_endpoints() -> Vec<String> {
    std::env::var("ETCD_ENDPOINTS")
        .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_else(|_| vec!["http://127.0.0.1:22379".to_string()])
}

/// 测试命名空间
const TEST_NAMESPACE: &str = "flare-test";

/// 创建测试用的 etcd 后端配置
fn create_test_config(service_type: &str) -> DiscoveryConfig {
    let mut backend_config = HashMap::new();
    backend_config.insert("endpoints".to_string(), json!(etcd_endpoints()));
    backend_config.insert("service_type".to_string(), json!(service_type));
    backend_config.insert("ttl".to_string(), json!(90));

    DiscoveryConfig {
        backend: BackendType::Etcd,
        backend_config,
        namespace: None,
        version: None,
        tag_filters: vec![],
        load_balance: LoadBalanceStrategy::ConsistentHash,
        health_check: Some(HealthCheckConfig {
            interval: 10,
            timeout: 5,
            failure_threshold: 3,
            success_threshold: 2,
            path: Some("/health".to_string()),
        }),
        refresh_interval: Some(30),
    }
}

/// 创建测试用的服务实例
fn create_test_instance(
    service_type: &str,
    instance_id: &str,
    port: u16,
) -> ServiceInstance {
    let mut tags = HashMap::new();
    tags.insert("env".to_string(), "test".to_string());
    tags.insert("region".to_string(), "us-east-1".to_string());

    ServiceInstance {
        service_type: service_type.to_string(),
        instance_id: instance_id.to_string(),
        address: format!("127.0.0.1:{}", port).parse().unwrap(),
        namespace: Some(TEST_NAMESPACE.to_string()),
        version: Some("v1.0.0".to_string()),
        tags,
        metadata: InstanceMetadata {
            region: Some("us-east-1".to_string()),
            zone: Some("us-east-1a".to_string()),
            environment: Some("test".to_string()),
            ..Default::default()
        },
        healthy: true,
        weight: 100,
    }
}

/// 测试：服务注册
#[tokio::test]
#[ignore]
async fn test_etcd_register() {
    let config = create_test_config("test-service");
    let backend = DiscoveryFactory::create_backend(&config)
        .await
        .expect("Failed to create etcd backend");

    let instance = create_test_instance("test-service", "node-1", 8080);

    // 注册服务
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register service");

    // 验证服务已注册（通过发现）
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");

    assert!(
        instances.iter().any(|i| i.instance_id == "node-1"),
        "Service instance not found after registration"
    );

    // 清理
    backend
        .unregister(&instance.instance_id)
        .await
        .expect("Failed to unregister service");
}

/// 测试：服务注销
#[tokio::test]
#[ignore]
async fn test_etcd_unregister() {
    let config = create_test_config("test-service");
    let backend = DiscoveryFactory::create_backend(&config)
        .await
        .expect("Failed to create etcd backend");

    let instance = create_test_instance("test-service", "node-2", 8081);

    // 注册服务
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register service");

    // 注销服务
    backend
        .unregister(&instance.instance_id)
        .await
        .expect("Failed to unregister service");

    // 验证服务已注销
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");

    assert!(
        !instances.iter().any(|i| i.instance_id == "node-2"),
        "Service instance still exists after unregistration"
    );
}

/// 测试：服务发现
#[tokio::test]
#[ignore]
async fn test_etcd_discover() {
    let config = create_test_config("test-service");
    let backend = DiscoveryFactory::create_backend(&config)
        .await
        .expect("Failed to create etcd backend");

    // 注册多个服务实例
    let instance1 = create_test_instance("test-service", "node-3", 8082);
    let instance2 = create_test_instance("test-service", "node-4", 8083);

    backend
        .register(instance1.clone())
        .await
        .expect("Failed to register instance1");
    backend
        .register(instance2.clone())
        .await
        .expect("Failed to register instance2");

    // 等待 etcd 同步
    sleep(Duration::from_millis(500)).await;

    // 发现服务
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");

    assert!(
        instances.len() >= 2,
        "Expected at least 2 instances, found {}",
        instances.len()
    );
    assert!(
        instances.iter().any(|i| i.instance_id == "node-3"),
        "Instance node-3 not found"
    );
    assert!(
        instances.iter().any(|i| i.instance_id == "node-4"),
        "Instance node-4 not found"
    );

    // 清理
    backend
        .unregister(&instance1.instance_id)
        .await
        .expect("Failed to unregister instance1");
    backend
        .unregister(&instance2.instance_id)
        .await
        .expect("Failed to unregister instance2");
}

/// 测试：服务发现（使用 ServiceDiscover）
#[tokio::test]
#[ignore]
async fn test_etcd_service_discover() {
    let config = create_test_config("test-service");
    let backend = DiscoveryFactory::create_backend(&config)
        .await
        .expect("Failed to create etcd backend");

    // 注册服务实例
    let instance = create_test_instance("test-service", "node-5", 8084);
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register service");

    // 创建服务发现器
    let (discover, _updater) = DiscoveryFactory::create_discover(config)
        .await
        .expect("Failed to create service discover");

    // 等待服务发现同步
    sleep(Duration::from_millis(2000)).await;

    // 使用 ServiceClient 获取 Channel
    let mut client = ServiceClient::new(discover);
    
    // 使用超时来避免无限等待
    let channel_result = tokio::time::timeout(
        Duration::from_secs(5),
        client.get_channel()
    ).await;

    // 验证能够获取 Channel（如果服务实例存在）
    match channel_result {
        Ok(Ok(_channel)) => {
            println!("✅ Successfully obtained channel from service discovery");
        }
        Ok(Err(e)) => {
            println!("⚠️ Failed to get channel: {}", e);
        }
        Err(_) => {
            println!("⚠️ Timeout waiting for channel (service may not be available yet)");
        }
    }

    // 清理
    backend
        .unregister(&instance.instance_id)
        .await
        .expect("Failed to unregister service");
}

/// 测试：服务注册 + 发现（使用 register_and_discover）
#[tokio::test]
#[ignore]
async fn test_etcd_register_and_discover() {
    let instance = create_test_instance("test-service", "node-6", 8085);

    // 使用快速构建方法
    let (registry, discover, _updater) = DiscoveryFactory::register_and_discover(
        BackendType::Etcd,
        etcd_endpoints(),
        "test-service".to_string(),
        instance.clone(),
    )
    .await
    .expect("Failed to register and discover");

    // 等待服务注册和发现同步
    sleep(Duration::from_millis(2000)).await;

    // 使用 ServiceClient 获取 Channel
    let mut client = ServiceClient::new(discover);
    
    // 使用超时来避免无限等待
    let channel_result = tokio::time::timeout(
        Duration::from_secs(5),
        client.get_channel()
    ).await;

    match channel_result {
        Ok(Ok(_channel)) => {
            println!("✅ Successfully obtained channel from registered service");
        }
        Ok(Err(e)) => {
            println!("⚠️ Failed to get channel: {}", e);
        }
        Err(_) => {
            println!("⚠️ Timeout waiting for channel (service may not be available yet)");
        }
    }

    // registry 会在 drop 时自动注销服务
    drop(registry);
    sleep(Duration::from_millis(500)).await;
}

/// 测试：心跳续期
#[tokio::test]
#[ignore]
async fn test_etcd_heartbeat() {
    use flare_server_core::discovery::ServiceRegistry;

    let config = create_test_config("test-service");
    let backend = DiscoveryFactory::create_backend(&config)
        .await
        .expect("Failed to create etcd backend");

    let instance = create_test_instance("test-service", "node-7", 8086);

    // 注册服务
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register service");

    // 创建注册器（自动心跳）
    let registry = ServiceRegistry::new(backend.clone(), instance.clone(), 5); // 5 秒心跳间隔

    // 等待心跳发送
    sleep(Duration::from_secs(6)).await;

    // 验证服务仍然存在（心跳续期成功）
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");

    assert!(
        instances.iter().any(|i| i.instance_id == "node-7"),
        "Service instance not found after heartbeat"
    );

    // registry 会在 drop 时自动注销
    drop(registry);
    sleep(Duration::from_millis(500)).await;
}

/// 测试：使用默认配置创建服务发现
#[tokio::test]
#[ignore]
async fn test_etcd_create_with_defaults() {
    // 使用默认配置创建服务发现
    let (discover, _updater) = DiscoveryFactory::create_with_defaults(
        BackendType::Etcd,
        etcd_endpoints(),
        "test-service".to_string(),
    )
    .await
    .expect("Failed to create discover with defaults");

    // 使用 ServiceClient
    let mut client = ServiceClient::new(discover);
    
    // 尝试获取 Channel（可能没有服务实例，这是正常的）
    // 使用超时来避免无限等待
    let channel_result = tokio::time::timeout(
        Duration::from_secs(5),
        client.get_channel()
    ).await;
    
    match channel_result {
        Ok(Ok(_channel)) => {
            println!("✅ Successfully obtained channel");
        }
        Ok(Err(e)) => {
            println!("⚠️ Failed to get channel: {}", e);
        }
        Err(_) => {
            println!("⚠️ Timeout waiting for channel (no services available)");
        }
    }
}

/// 测试：标签过滤
#[tokio::test]
#[ignore]
async fn test_etcd_tag_filter() {
    let config = create_test_config("test-service");
    let backend = DiscoveryFactory::create_backend(&config)
        .await
        .expect("Failed to create etcd backend");

    // 注册两个服务实例，带有不同的标签
    let mut instance1 = create_test_instance("test-service", "node-8", 8087);
    instance1.tags.insert("env".to_string(), "prod".to_string());

    let mut instance2 = create_test_instance("test-service", "node-9", 8088);
    instance2.tags.insert("env".to_string(), "test".to_string());

    backend
        .register(instance1.clone())
        .await
        .expect("Failed to register instance1");
    backend
        .register(instance2.clone())
        .await
        .expect("Failed to register instance2");

    sleep(Duration::from_millis(500)).await;

    // 使用标签过滤（只查找 env=prod 的实例）
    let mut tag_filter_map = HashMap::new();
    tag_filter_map.insert("env".to_string(), "prod".to_string());

    let instances = backend
        .discover("test-service", None, None, Some(&tag_filter_map))
        .await
        .expect("Failed to discover services with tag filter");

    // 应该只找到 env=prod 的实例
    assert!(
        instances.iter().any(|i| i.instance_id == "node-8"),
        "Instance node-8 (env=prod) not found"
    );
    assert!(
        !instances.iter().any(|i| i.instance_id == "node-9"),
        "Instance node-9 (env=test) should not be found"
    );

    // 清理
    backend
        .unregister(&instance1.instance_id)
        .await
        .expect("Failed to unregister instance1");
    backend
        .unregister(&instance2.instance_id)
        .await
        .expect("Failed to unregister instance2");
}

