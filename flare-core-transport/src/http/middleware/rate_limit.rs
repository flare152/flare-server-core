//! HTTP 限流中间件

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// 简单的限流器
pub struct RateLimiter {
    requests_per_second: u32,
    burst_capacity: u32,
    clients: Arc<Mutex<HashMap<String, ClientState>>>,
}

struct ClientState {
    tokens: u32,
    last_update: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: u32, burst_capacity: u32) -> Self {
        Self {
            requests_per_second,
            burst_capacity,
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn check(&self, client_id: &str) -> bool {
        let mut clients = self.clients.lock().await;
        let now = Instant::now();

        let state = clients.entry(client_id.to_string()).or_insert(ClientState {
            tokens: self.burst_capacity,
            last_update: now,
        });

        // 补充令牌
        let elapsed = now.duration_since(state.last_update);
        let tokens_to_add = (elapsed.as_secs_f64() * self.requests_per_second as f64) as u32;
        state.tokens = (state.tokens + tokens_to_add).min(self.burst_capacity);
        state.last_update = now;

        // 检查并消耗令牌
        if state.tokens > 0 {
            state.tokens -= 1;
            true
        } else {
            false
        }
    }
}

/// 限流层
#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: Arc<RateLimiter>,
}

impl RateLimitLayer {
    pub fn new(requests_per_second: u32, burst_capacity: u32) -> Self {
        Self {
            limiter: Arc::new(RateLimiter::new(requests_per_second, burst_capacity)),
        }
    }

    pub fn limiter(&self) -> Arc<RateLimiter> {
        self.limiter.clone()
    }
}
