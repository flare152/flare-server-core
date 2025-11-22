//! 工具函数模块

use std::net::SocketAddr;
use tonic::{Request, Status};

/// 提取用户ID
pub fn extract_user_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 提取追踪ID
pub fn extract_trace_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 提取请求ID
pub fn extract_request_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 提取设备ID
pub fn extract_device_id<T>(req: &Request<T>) -> Option<String> {
    req.metadata()
        .get("x-device-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// 创建追踪元数据
pub fn create_traced_metadata(
    trace_id: &str,
    request_id: &str,
) -> Result<tonic::metadata::MetadataMap, Status> {
    let mut metadata = tonic::metadata::MetadataMap::new();

    metadata.insert(
        "x-trace-id",
        trace_id
            .parse()
            .map_err(|_| Status::internal("Invalid trace_id"))?,
    );

    metadata.insert(
        "x-request-id",
        request_id
            .parse()
            .map_err(|_| Status::internal("Invalid request_id"))?,
    );

    Ok(metadata)
}

/// 错误转换为 Status
pub fn error_to_status(error: impl std::error::Error, code: tonic::Code) -> Status {
    Status::new(code, error.to_string())
}

/// 等待服务启动就绪（通过 TCP 连接重试）
///
/// 使用指数退避策略重试连接，直到服务真正可以接受连接。
/// 这是一个通用的服务启动就绪检测工具，适用于 gRPC、HTTP 等任何基于 TCP 的服务。
///
/// # 参数
/// * `address` - 服务地址
///
/// # 返回
/// * `Ok(())` - 服务已就绪
/// * `Err` - 超时或连接失败
///
/// # 示例
/// ```rust,no_run
/// use flare_server_core::utils::wait_for_server_ready;
/// use std::net::SocketAddr;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let address: SocketAddr = "127.0.0.1:50051".parse()?;
/// wait_for_server_ready(address).await?;
/// // 服务已就绪，可以进行服务注册等操作
/// # Ok(())
/// # }
/// ```
pub async fn wait_for_server_ready(address: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::time::Duration;
    use tokio::net::TcpStream;
    use tokio::time::{sleep, timeout};
    use tracing::debug;

    const MAX_RETRIES: u32 = 30; // 最多重试 30 次
    const INITIAL_DELAY_MS: u64 = 50; // 初始延迟 50ms
    const MAX_DELAY_MS: u64 = 500; // 最大延迟 500ms
    const TOTAL_TIMEOUT_SECS: u64 = 10; // 总超时时间 10 秒

    let start = std::time::Instant::now();
    let mut delay_ms = INITIAL_DELAY_MS;

    for attempt in 1..=MAX_RETRIES {
        // 检查总超时时间
        if start.elapsed().as_secs() > TOTAL_TIMEOUT_SECS {
            return Err(format!(
                "Server readiness check timeout after {} seconds",
                TOTAL_TIMEOUT_SECS
            ).into());
        }

        // 尝试连接
        match timeout(Duration::from_millis(100), TcpStream::connect(address)).await {
            Ok(Ok(_)) => {
                // 连接成功，服务已就绪
                debug!(
                    address = %address,
                    attempts = attempt,
                    elapsed_ms = start.elapsed().as_millis(),
                    "Server is ready"
                );
                return Ok(());
            }
            Ok(Err(e)) => {
                // 连接失败，继续重试
                debug!(
                    address = %address,
                    attempt = attempt,
                    error = %e,
                    "Connection attempt failed, retrying..."
                );
            }
            Err(_) => {
                // 连接超时，继续重试
                debug!(
                    address = %address,
                    attempt = attempt,
                    "Connection attempt timed out, retrying..."
                );
            }
        }

        // 指数退避：延迟时间逐渐增加，但不超过最大值
        sleep(Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * 2).min(MAX_DELAY_MS);
    }

    Err(format!(
        "Server readiness check failed after {} attempts",
        MAX_RETRIES
    ).into())
}
