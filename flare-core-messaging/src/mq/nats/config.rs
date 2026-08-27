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
    /// 单个 stream 的落盘上限（字节）。
    ///
    /// 必须有：`RetentionPolicy::Limits` 下只按 `max_age` 淘汰，一个高流量 stream 会一路吃到
    /// 服务端 `max_file_store` 配额上限，之后**所有 publish** 返回
    /// `insufficient resources (10023)`。对 IM 来说这等于全站发消息中断，且没有告警。
    /// 配合 `DiscardPolicy::Old` 使用：满了丢最旧的（基本都是已被消费的），而不是拒绝新消息。
    pub max_bytes: i64,
}

impl NatsStreamSpec {
    pub fn new(name: impl Into<String>, subjects: Vec<String>) -> Self {
        Self {
            name: name.into(),
            subjects,
            max_age: Duration::from_secs(7 * 24 * 3600),
            duplicate_window: Duration::from_secs(10 * 60),
            num_replicas: 1,
            max_bytes: DEFAULT_STREAM_MAX_BYTES,
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

    pub fn with_max_bytes(mut self, max_bytes: i64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn with_num_replicas(mut self, num_replicas: usize) -> Self {
        self.num_replicas = num_replicas.clamp(1, 5);
        self
    }
}

/// 单 stream 默认落盘上限 2 GiB。三条默认流合计 6 GiB，低于部署默认的
/// `NATS_MAX_FILE_STORE=10GB`，留出余量给去重窗口与索引。
pub const DEFAULT_STREAM_MAX_BYTES: i64 = 2 * 1024 * 1024 * 1024;

pub const STREAM_FLARE_MESSAGE: &str = "FLARE_MESSAGE";
pub const STREAM_FLARE_PUSH: &str = "FLARE_PUSH";
pub const STREAM_FLARE_DLQ: &str = "FLARE_DLQ";

/// 死信(DLQ)统一 subject 前缀。死信流用通配 `flare.im.dlq.>` 单独成 stream,**不与任何消费者订阅的
/// subject 重叠**(否则死信会被原消费者再次消费→无限重投/再死信循环)。各服务死信投到 `flare.im.dlq.<service>`。
pub const SUBJECT_FLARE_DLQ_PREFIX: &str = "flare.im.dlq";

/// 死信流规格:捕获所有处理失败/毒消息,长留存(7天)以便排查与重放。通配 `flare.im.dlq.>` 覆盖各服务
/// DLQ 子 subject,且与任何消费者订阅的 subject 都不重叠(避免死信被重新消费形成循环)。
pub fn dlq_stream_spec() -> NatsStreamSpec {
    NatsStreamSpec::new(STREAM_FLARE_DLQ, vec!["flare.im.dlq.>".to_string()])
        .with_max_age(Duration::from_secs(7 * 24 * 3600))
}

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
        dlq_stream_spec(),
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

    /// JetStream explicit ack 等待时间（秒）。
    fn ack_wait_secs(&self) -> u64 {
        30
    }

    /// JetStream 最大投递次数。
    fn max_deliver(&self) -> i64 {
        16
    }

    /// JetStream 最大未 ACK 消息数。
    fn max_ack_pending(&self) -> i64 {
        (self.batch_size().saturating_mul(16)).max(1024) as i64
    }

    /// 是否启用持久化
    fn enable_durable(&self) -> bool;

    /// 新建的 durable 是否从"此刻之后"开始投递，而不是重放整条流的历史。
    ///
    /// 默认 false（JetStream 的 DeliverAll），保持"不丢消息"的语义。
    /// 但 durable 名字里含订阅的 subject 列表：给一个已上线的消费者**加 subject**
    /// 会生成一个全新的 durable，于是它会把流里几十万条历史全部重放一遍。
    /// 当消费副作用不幂等（例如未读计数自增）时，这会直接把数据写坏。
    /// 这类"只关心新增量"的消费者应当返回 true。
    fn deliver_from_new(&self) -> bool {
        false
    }

    /// All streams this consumer may subscribe to.
    fn stream_specs(&self) -> Vec<NatsStreamSpec> {
        default_stream_specs()
    }
}

#[cfg(test)]
mod tests {
    use super::{default_stream_specs, subject_matches};

    /// 每条默认流都必须有字节上限。
    ///
    /// 没有上限时 stream 会一路吃满服务端 `max_file_store` 配额，之后所有 publish 返回
    /// `insufficient resources (10023)`——线上表现是消息一直转圈发不出去，日志里没有任何
    /// ERROR 级信号。这个断言是唯一能提前发现的地方。
    #[test]
    fn every_default_stream_has_a_byte_cap() {
        for spec in default_stream_specs() {
            assert!(
                spec.max_bytes > 0,
                "stream {} 没有 max_bytes，写满后会让全站发消息中断",
                spec.name
            );
        }
    }

    /// 所有默认流的上限之和必须留在部署默认配额（NATS_MAX_FILE_STORE=10GB）之内，
    /// 否则单条流仍能把整个 JetStream 存储吃穿，`DiscardPolicy::Old` 也救不了别的流。
    #[test]
    fn default_stream_caps_fit_in_deploy_file_store() {
        const DEPLOY_MAX_FILE_STORE: i64 = 10 * 1024 * 1024 * 1024;
        let total: i64 = default_stream_specs().iter().map(|s| s.max_bytes).sum();
        assert!(
            total < DEPLOY_MAX_FILE_STORE,
            "默认流上限合计 {total} 字节，超过部署配额 {DEPLOY_MAX_FILE_STORE}"
        );
    }

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
