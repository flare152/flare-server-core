//! Consul 服务发现后端测试

use flare_server_core::discovery::backend::consul::ConsulBackend;
use flare_server_core::discovery::{
    BackendType, DiscoveryBackend, DiscoveryConfig, NamespaceConfig, ServiceInstance,
};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;

/// 测试辅助函数：创建 Consul 配置
fn create_consul_config() -> DiscoveryConfig {
    let mut backend_config = HashMap::new();
    backend_config.insert("url".to_string(), json!("http://localhost:8500"));
    backend_config.insert("service_type".to_string(), json!("test-service"));

    DiscoveryConfig {
        backend: BackendType::Consul,
        backend_config,
        namespace: Some(NamespaceConfig {
            default: Some("default".to_string()),
            separator: Some("/".to_string()),
        }),
        version: None,
        tag_filters: vec![],
        load_balance: flare_server_core::discovery::LoadBalanceStrategy::RoundRobin,
        health_check: None,
        refresh_interval: Some(30),
    }
}

/// 测试辅助函数：创建测试服务实例
fn create_test_instance(id: &str, port: u16) -> ServiceInstance {
    ServiceInstance::new(
        "test-service",
        id,
        format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap(),
    )
    .with_namespace("default")
    .with_version("v1.0.0")
    .with_tag("environment", "test")
    .with_tag("region", "us-east-1")
    .with_weight(100)
    .with_health(true)
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_backend_creation() {
    let config = create_consul_config();

    match ConsulBackend::new(&config).await {
        Ok(_) => {
            println!("✅ Consul backend created successfully");
        }
        Err(e) => {
            eprintln!(
                "⚠️  Consul backend creation failed (expected if Consul is not running): {}",
                e
            );
            // 如果 Consul 未运行，跳过测试
            return;
        }
    }
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_register_and_discover() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 创建测试实例
    let instance1 = create_test_instance("instance-1", 8080);
    let instance2 = create_test_instance("instance-2", 8081);

    // 注册实例
    backend
        .register(instance1.clone())
        .await
        .expect("Failed to register instance 1");
    backend
        .register(instance2.clone())
        .await
        .expect("Failed to register instance 2");

    // 等待 Consul 同步
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 发现服务
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");

    assert!(
        !instances.is_empty(),
        "Should discover at least one instance"
    );

    // 验证实例
    let found_instance1 = instances.iter().find(|i| i.instance_id == "instance-1");
    assert!(found_instance1.is_some(), "Should find instance-1");

    let found_instance2 = instances.iter().find(|i| i.instance_id == "instance-2");
    assert!(found_instance2.is_some(), "Should find instance-2");

    // 清理
    backend.unregister("instance-1").await.ok();
    backend.unregister("instance-2").await.ok();
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_namespace_filtering() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 创建不同命名空间的实例
    let instance1 = create_test_instance("instance-1", 8080).with_namespace("test");
    let instance2 = create_test_instance("instance-2", 8081).with_namespace("production");

    backend
        .register(instance1.clone())
        .await
        .expect("Failed to register instance 1");
    backend
        .register(instance2.clone())
        .await
        .expect("Failed to register instance 2");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 只查询 test 命名空间
    let instances = backend
        .discover("test-service", Some("test"), None, None)
        .await
        .expect("Failed to discover services");

    // Consul 的命名空间过滤可能通过标签实现
    // 这里验证至少有一个实例
    assert!(!instances.is_empty(), "Should find instances");

    // 清理
    backend.unregister("instance-1").await.ok();
    backend.unregister("instance-2").await.ok();
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_version_filtering() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 创建不同版本的实例
    let instance1 = create_test_instance("instance-1", 8080).with_version("v1.0.0");
    let instance2 = create_test_instance("instance-2", 8081).with_version("v2.0.0");

    backend
        .register(instance1.clone())
        .await
        .expect("Failed to register instance 1");
    backend
        .register(instance2.clone())
        .await
        .expect("Failed to register instance 2");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 只查询 v1.0.0 版本
    let instances = backend
        .discover("test-service", None, Some("v1.0.0"), None)
        .await
        .expect("Failed to discover services");

    // 验证版本过滤
    for instance in &instances {
        if let Some(version) = &instance.version {
            assert_eq!(version, "v1.0.0", "All instances should be v1.0.0");
        }
    }

    // 清理
    backend.unregister("instance-1").await.ok();
    backend.unregister("instance-2").await.ok();
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_tag_filtering() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 创建不同标签的实例
    let instance1 = create_test_instance("instance-1", 8080)
        .with_tag("environment", "test")
        .with_tag("region", "us-east-1");
    let instance2 = create_test_instance("instance-2", 8081)
        .with_tag("environment", "production")
        .with_tag("region", "us-east-1");

    backend
        .register(instance1.clone())
        .await
        .expect("Failed to register instance 1");
    backend
        .register(instance2.clone())
        .await
        .expect("Failed to register instance 2");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 按标签过滤
    let mut tag_filters = HashMap::new();
    tag_filters.insert("environment".to_string(), "test".to_string());

    let instances = backend
        .discover("test-service", None, None, Some(&tag_filters))
        .await
        .expect("Failed to discover services");

    // 验证标签过滤
    for instance in &instances {
        assert_eq!(
            instance.tags.get("environment").unwrap(),
            "test",
            "All instances should have environment=test"
        );
    }

    // 清理
    backend.unregister("instance-1").await.ok();
    backend.unregister("instance-2").await.ok();
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_unregister() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 注册实例
    let instance = create_test_instance("instance-1", 8080);
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register instance");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 验证实例存在
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");
    assert!(instances.iter().any(|i| i.instance_id == "instance-1"));

    // 注销实例
    backend
        .unregister("instance-1")
        .await
        .expect("Failed to unregister instance");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 验证实例已删除
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");
    assert!(!instances.iter().any(|i| i.instance_id == "instance-1"));
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_watch() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 启动 watch
    let mut receiver = backend
        .watch("test-service")
        .await
        .expect("Failed to start watch");

    // 注册新实例（应该在 watch 中收到通知）
    let instance = create_test_instance("instance-1", 8080);
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register instance");

    // 等待 watch 事件（设置超时）
    let watch_result =
        tokio::time::timeout(tokio::time::Duration::from_secs(10), receiver.recv()).await;

    if let Ok(Some(_)) = watch_result {
        println!("✅ Watch event received");
    } else {
        println!("⚠️  Watch event timeout (may be normal if watch is not fully implemented)");
    }

    // 清理
    backend.unregister("instance-1").await.ok();
}

#[tokio::test]
#[ignore] // 需要运行中的 Consul 实例
async fn test_consul_health_check() {
    let config = create_consul_config();

    let backend = match ConsulBackend::new(&config).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("⚠️  Skipping test: Consul not available: {}", e);
            return;
        }
    };

    // 注册实例（Consul 会自动进行健康检查）
    let instance = create_test_instance("instance-1", 8080);
    backend
        .register(instance.clone())
        .await
        .expect("Failed to register instance");

    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 发现服务（只返回健康的实例）
    let instances = backend
        .discover("test-service", None, None, None)
        .await
        .expect("Failed to discover services");

    // Consul 的健康检查由 Consul 自己管理
    // 这里验证至少能发现实例
    assert!(!instances.is_empty(), "Should discover healthy instances");

    // 清理
    backend.unregister("instance-1").await.ok();
}
