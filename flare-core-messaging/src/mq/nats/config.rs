//! NATS JetStream 配置 Trait

use std::time::Duration;

/// JetStream stream 与其拥有的 subjects。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsStreamSpec {
    pub name: String,
    pub subjects: Vec<String>,
    /// 消息保留时长。核心消息流默认保留 7 天，推送任务流默认 1 天。
    pub max_age: Duration,
    /// JetStream 去重窗口。生产者使用稳定 message_id 时，窗口内重试不会重复落盘。
    pub duplicate_window: Duration,
    /// JetStream 集群副本数。单机开发环境保持 1，生产建议使用 3。
    pub num_replicas: usize,
}

impl NatsStreamSpec {
    pub fn new(name: impl Into<String>, subjects: Vec<String>) -> Self {
        Self {
            name: name.into(),
            subjects,
            max_age: Duration::from_secs(7 * 24 * 3600),
            duplicate_window: Duration::from_secs(10 * 60),
            num_replicas: 1,
        }
    }

    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    pub fn with_duplicate_window(mut self, duplicate_window: Duration) -> Self {
        self.duplicate_window = duplicate_window;
        self
    }

    pub fn with_num_replicas(mut self, num_replicas: usize) -> Self {
        self.num_replicas = num_replicas.clamp(1, 5);
        self
    }
}

pub const STREAM_FLARE_MESSAGE: &str = "FLARE_MESSAGE";
pub const STREAM_FLARE_PUSH: &str = "FLARE_PUSH";

/// IM 默认 JetStream 拓扑。业务只使用 subject，stream 归属由 core 统一解析。
pub fn default_stream_specs() -> Vec<NatsStreamSpec> {
    vec![
        NatsStreamSpec::new(
            STREAM_FLARE_MESSAGE,
            vec![
                "flare.im.message.*".to_string(),
                "flare.im.conversation.*".to_string(),
            ],
        ),
        NatsStreamSpec::new(STREAM_FLARE_PUSH, vec!["flare.im.push.*".to_string()])
            .with_max_age(Duration::from_secs(24 * 3600)),
    ]
}

pub fn subject_matches(pattern: &str, subject: &str) -> bool {
    if pattern == subject {
        return true;
    }

    let pattern_tokens = pattern.split('.').collect::<Vec<_>>();
    let subject_tokens = subject.split('.').collect::<Vec<_>>();

    let mut subject_idx = 0usize;
    for (idx, token) in pattern_tokens.iter().enumerate() {
        if *token == ">" {
            return idx == pattern_tokens.len() - 1 && subject_idx < subject_tokens.len();
        }

        let Some(subject_token) = subject_tokens.get(subject_idx) else {
            return false;
        };

        if *token != "*" && *token != *subject_token {
            return false;
        }

        subject_idx += 1;
    }

    subject_idx == subject_tokens.len()
}

pub fn resolve_subject_stream<'a>(
    specs: &'a [NatsStreamSpec],
    subject: &str,
) -> Option<&'a NatsStreamSpec> {
    specs.iter().find(|spec| {
        spec.subjects
            .iter()
            .any(|pattern| subject_matches(pattern, subject))
    })
}

/// NATS JetStream 生产者配置 Trait
pub trait NatsProducerConfig: Send + Sync {
    /// NATS 服务器 URL
    fn nats_url(&self) -> &str;

    /// 超时时间（毫秒）
    fn timeout_ms(&self) -> u64;

    /// 重试次数
    fn retries(&self) -> u32;

    /// 重试退避时间（毫秒）
    fn retry_backoff_ms(&self) -> u64;

    /// All streams this producer may publish to.
    fn stream_specs(&self) -> Vec<NatsStreamSpec> {
        default_stream_specs()
    }
}

/// NATS JetStream 消费者配置 Trait
pub trait NatsConsumerConfig: Send + Sync {
    /// NATS 服务器 URL
    fn nats_url(&self) -> &str;

    /// Consumer group ID
    fn consumer_group(&self) -> &str;

    /// 是否启用手动确认
    fn enable_manual_ack(&self) -> bool;

    /// 批处理大小
    fn batch_size(&self) -> usize;

    /// 批处理超时（毫秒）
    fn batch_timeout_ms(&self) -> u64;

    /// 是否启用持久化
    fn enable_durable(&self) -> bool;

    /// All streams this consumer may subscribe to.
    fn stream_specs(&self) -> Vec<NatsStreamSpec> {
        default_stream_specs()
    }
}

#[cfg(test)]
mod tests {
    use super::subject_matches;

    #[test]
    fn matches_nats_single_token_wildcard() {
        assert!(subject_matches("flare.im.push.*", "flare.im.push.online"));
        assert!(!subject_matches(
            "flare.im.push.*",
            "flare.im.push.online.extra"
        ));
        assert!(!subject_matches("flare.im.push.*", "push-online"));
    }

    #[test]
    fn matches_nats_multi_token_wildcard() {
        assert!(subject_matches("flare.im.>", "flare.im.push.online"));
        assert!(!subject_matches("flare.im.>", "flare.im"));
        assert!(!subject_matches("flare.im.>", "flare"));
    }
}
