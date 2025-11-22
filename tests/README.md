# Discovery 模块测试

本目录包含基于 etcd 和 Consul 的服务发现后端集成测试。

## 测试文件

- `etcd_backend_test.rs` - etcd 后端测试
- `consul_backend_test.rs` - Consul 后端测试

## 前置条件

### etcd 测试

需要运行 etcd 服务器实例。可以通过以下方式启动：

#### 使用 Docker（推荐）

```bash
docker run -d --name etcd-test -p 2379:2379 -p 2380:2380 \
  quay.io/coreos/etcd:v3.5.9 \
  etcd --advertise-client-urls=http://127.0.0.1:2379 \
       --listen-client-urls=http://0.0.0.0:2379
```

#### 使用本地安装的 etcd

```bash
etcd --advertise-client-urls=http://127.0.0.1:2379 \
     --listen-client-urls=http://0.0.0.0:2379
```

### Consul 测试

需要运行 Consul 服务器实例。可以通过以下方式启动：

#### 使用 Docker（推荐）

```bash
docker run -d --name consul-test -p 8500:8500 consul:latest
```

#### 使用本地安装的 Consul

```bash
consul agent -dev -client=0.0.0.0
```

## 运行测试

默认情况下，所有测试都被标记为 `#[ignore]`，以避免在没有运行服务发现后端的情况下运行测试。

### 运行所有测试

```bash
# 运行 etcd 测试
cargo test --test etcd_backend_test -- --ignored

# 运行 Consul 测试
cargo test --test consul_backend_test -- --ignored
```

### 运行特定测试

```bash
# 运行 etcd 服务注册测试
cargo test --test etcd_backend_test test_etcd_register -- --ignored

# 运行 Consul 服务发现测试
cargo test --test consul_backend_test test_consul_discover -- --ignored
```

### 使用环境变量配置

#### etcd 测试

```bash
# 指定 etcd endpoints（多个用逗号分隔）
ETCD_ENDPOINTS=http://127.0.0.1:2379,http://127.0.0.1:2378 \
  cargo test --test etcd_backend_test -- --ignored
```

#### Consul 测试

```bash
# 指定 Consul URL
CONSUL_URL=http://127.0.0.1:8500 \
  cargo test --test consul_backend_test -- --ignored
```

## 测试覆盖范围

### etcd 后端测试

- ✅ 服务注册 (`test_etcd_register`)
- ✅ 服务注销 (`test_etcd_unregister`)
- ✅ 服务发现 (`test_etcd_discover`)
- ✅ 服务发现（使用 ServiceDiscover）(`test_etcd_service_discover`)
- ✅ 服务注册 + 发现（使用 register_and_discover）(`test_etcd_register_and_discover`)
- ✅ 心跳续期 (`test_etcd_heartbeat`)
- ✅ 使用默认配置创建服务发现 (`test_etcd_create_with_defaults`)
- ✅ 标签过滤 (`test_etcd_tag_filter`)

### Consul 后端测试

- ✅ 服务注册 (`test_consul_register`)
- ✅ 服务注销 (`test_consul_unregister`)
- ✅ 服务发现 (`test_consul_discover`)
- ✅ 服务发现（使用 ServiceDiscover）(`test_consul_service_discover`)
- ✅ 服务注册 + 发现（使用 register_and_discover）(`test_consul_register_and_discover`)
- ✅ 心跳续期 (`test_consul_heartbeat`)
- ✅ 使用默认配置创建服务发现 (`test_consul_create_with_defaults`)
- ✅ 标签过滤 (`test_consul_tag_filter`)
- ✅ 健康检查 (`test_consul_health_check`)

## CI/CD 集成

在 CI/CD 环境中，建议使用 Docker Compose 或 Kubernetes 来启动测试所需的服务：

```yaml
# docker-compose.test.yml
version: '3'
services:
  etcd:
    image: quay.io/coreos/etcd:v3.5.9
    ports:
      - "2379:2379"
    command: >
      etcd
      --advertise-client-urls=http://127.0.0.1:2379
      --listen-client-urls=http://0.0.0.0:2379
  
  consul:
    image: consul:latest
    ports:
      - "8500:8500"
```

然后运行测试：

```bash
docker-compose -f docker-compose.test.yml up -d
cargo test --test etcd_backend_test -- --ignored
cargo test --test consul_backend_test -- --ignored
docker-compose -f docker-compose.test.yml down
```

## 注意事项

1. **测试隔离**：每个测试都会创建独立的服务实例，使用不同的端口和实例 ID，避免冲突。

2. **同步等待**：etcd 和 Consul 的服务注册/注销需要一些时间才能同步。测试中已经包含了适当的等待时间。

3. **清理资源**：所有测试都会在结束时清理注册的服务实例，但建议在测试运行前确保 etcd/Consul 是干净的状态。

4. **并发测试**：这些测试不是为并发运行设计的。如果需要并发运行，请确保使用不同的命名空间或服务类型。

