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

/// 单 stream 默认落盘上限 1 GiB。
///
/// ⚠️ JetStream 把 `max_bytes` 当作**预留额**计入账户的 `max_file_store`——
/// 不是"用多少算多少"。所以这个默认值 × 流条数必须**严格小于**部署配额，
/// 否则最后建的那条流会直接失败，报的还是一句毫无指向性的
/// `insufficient storage resources (code 500, error code 10047)`，
/// 而对应的服务会进崩溃循环。
///
/// 这里定 1 GiB：三条默认流合计 3 GiB，在 4GB 起的部署配额下都还有余量。
/// 之前定 2 GiB（合计 6 GiB）只在 `NATS_MAX_FILE_STORE=10GB` 的默认部署下成立，
/// 一旦运维把配额调低（实测 6GB）就会把 push-server 打进崩溃循环。
pub const DEFAULT_STREAM_MAX_BYTES: i64 = 1024 * 1024 * 1024;

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
/// JetStream 的资源不足报错（存储 10047 / 一般资源 10023）翻译成能直接行动的信息。
///
/// 原始报错只有一句 `insufficient (storage) resources`，既不说是账户配额不够还是磁盘满了，
/// 也不会提 `max_bytes` 是**预留额**——JetStream 按上限把额度记在账户头上，不是用多少算多少。
/// 线上遇到过两次：一次是流写满导致**全站发消息静默中断**，一次是预留超配把 push-server
/// 打进崩溃循环。两次都因为报错毫无指向性而绕了远路。
///
/// 不是资源类错误时返回 `None`，调用方原样处理。
pub fn explain_jetstream_resource_error(what: &str, raw: &str) -> Option<String> {
    let low = raw.to_lowercase();
    if !low.contains("insufficient") && !raw.contains("10047") && !raw.contains("10023") {
        return None;
    }
    Some(format!(
        "{what} 失败：JetStream 资源不足（原始报错：{raw}）。\n\
         排查顺序：\n\
         1) 所有 stream 的 max_bytes **之和**是否小于服务端 jetstream.max_file_store\n\
            （部署里由 NATS_MAX_FILE_STORE 控制）——max_bytes 是按上限预留，不是按用量；\n\
         2) 该配额是否放得进宿主机剩余磁盘；\n\
         3) 流是否已写满且 discard 策略不是 Old（满了会拒绝新消息而不是丢旧的）。\n\
         注意这类失败**不会自愈**，重试只是延后暴露。"
    ))
}

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
    use super::{default_stream_specs, explain_jetstream_resource_error, subject_matches};

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

    /// 所有默认流的上限之和必须留在**运维可能设置的最小配额**之内。
    ///
    /// 判据一度对着仓库默认的 10GB 检查，于是 6 GiB 的合计轻松通过；
    /// 但运维把 `NATS_MAX_FILE_STORE` 调到 6GB 后，最后建的那条流直接建不出来，
    /// push-server 进崩溃循环（线上实测）。`max_bytes` 是**预留额**、按上限计入账户配额，
    /// 不是"用多少算多少"——所以这里必须按保守下限卡。
    /// JetStream 那句 `insufficient resources` 必须被翻译成能直接行动的信息。
    /// 线上两次事故（全站发消息静默中断 / push-server 崩溃循环）都是因为
    /// 原始报错既不说配额也不说磁盘，更不提 max_bytes 是预留额。
    #[test]
    fn resource_errors_are_explained_not_passed_through() {
        for raw in [
            "insufficient resources (code 503, error code 10023)",
            "jetstream error: insufficient storage resources available (code 500, error code 10047)",
        ] {
            let msg =
                explain_jetstream_resource_error("测试上下文", raw).expect("资源类报错必须被翻译");
            assert!(msg.contains("测试上下文"), "要带上出错的上下文");
            assert!(msg.contains(raw), "原始报错不能丢");
            assert!(msg.contains("max_bytes"), "要点明 max_bytes 是预留额");
            assert!(msg.contains("max_file_store"), "要指向该对照的配额项");
            assert!(msg.contains("不会自愈"), "要说明重试没用");
        }
    }

    /// 非资源类报错原样透传，不能被这层包装吃掉。
    #[test]
    fn non_resource_errors_pass_through_untouched() {
        assert!(explain_jetstream_resource_error("ctx", "connection refused").is_none());
        assert!(explain_jetstream_resource_error("ctx", "timeout").is_none());
    }

    #[test]
    fn default_stream_caps_fit_in_conservative_file_store() {
        // 单机小规格部署给 JetStream 的常见下限。
        const CONSERVATIVE_MAX_FILE_STORE: i64 = 4 * 1024 * 1024 * 1024;
        let total: i64 = default_stream_specs().iter().map(|s| s.max_bytes).sum();
        assert!(
            total < CONSERVATIVE_MAX_FILE_STORE,
            "默认流上限合计 {total} 字节，超过保守配额 {CONSERVATIVE_MAX_FILE_STORE}；\
             JetStream 按上限预留，超了会让最后建的那条流直接失败"
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
