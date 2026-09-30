# 变更记录

本文件只记录**已发布到 crates.io 的版本**。版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/)。

## 1.1.1 — 未发布

安全修复版本。1.1.0 发布于 2026-08-03，此后修掉的公告都在这里。
**建议所有 1.1.0 用户升级**。

### 不兼容改动

下列改动与 1.1.0 的公开 API 或 Redis 存储格式不兼容，升级时须同步调整调用方。

- `TokenStore` 的 `register` / `revoke` / `revoke_user` 增加 `tenant_id` 参数。
  本版本新增的 `TokenStore::publish_kick` 与 `TokenService::revoke_user` /
  `publish_kick` 也按租户定义：踢线信号是 `KickSignal`（租户、用户、可选设备与连接
  ID，以及网关关连接前下发给客户端的可选类型化原因 `reason`），频道载荷是它的 JSON，
  而不是裸 `user_id`。（bf15224）
- `RedisTokenStore` 的键改为 `{namespace}:token:v2:*`，用户集合键按租户与用户 ID
  分别编码。升级前登记的令牌不在新的用户集合里，`revoke_user` 撤不到它们，只能等其
  自然过期；升级前写下的单个令牌撤销位仍然生效（见下方“修复”）。（bf15224）
- `discovery::discover::ChannelService::new` 增加 `Endpoint` 参数。（c143f19）

### 安全

- `async-nats` 0.42 → 0.50，消除 `rustls-webpki` 0.102 的**四条公告**。
  依赖是间接引入的，升级 `async-nats` 是唯一能把它带上去的路径。
- 普通访问校验只接受 access 令牌：`CoreJwtTokenValidator` 和 `CoreJwtTokenIssuer`
  的刷新宽限路径遇到 `token_use` 为 refresh / control / admin 等非 access 用途的令牌
  一律返回 `Forbidden`，链式校验器也不再把这类令牌转交三方 Hook；未带 `token_use`
  的旧 access 令牌照常接受。新增 `TokenService::validate_access_token`。（bf15224）
- 令牌登记、撤销与踢线按租户隔离，不同租户下的同名用户不会被互相撤销或踢下线。
  （bf15224）
- 应用凭据绑定租户：`AppCredential` 增加 `tenant_id`，即该应用在数据面的权威租户，
  签发令牌时应以它为准而不是采信请求里的租户。环境变量格式为
  `app_id:tenant_id:secret`（此前未发布的两段式 `app_id:secret` 不再接受），重复的
  `app_id` 直接拒绝；解析错误不再回显输入，避免格式写错时把密钥带进日志。（f887afe）

### 新增

- Ed25519 令牌：`TokenService::from_ed25519_private_key`（签发端）与
  `from_ed25519_public_key`（校验端，只持公钥，任何签发调用都会失败）；
  `generate_with_ed25519` 签发必带用途与受众的令牌，`validate_ed25519_with_use`
  固定 EdDSA，严格匹配 issuer / audience / `token_use`，检查 exp / iat / jti / sub，
  过期没有宽限，配置了 `TokenStore` 时同时检查撤销。密钥接受 PKCS#8 私钥 PEM、SPKI
  公钥 PEM 或其 base64 PEM/DER 形式，错误信息不含密钥正文。HS256 的原有签发与刷新接口
  行为不变。见 `docs/ed25519-tokens.md`。（6eace59）
- Cell 内部签名上下文：`SignedContextSigner` / `SignedContextVerifier`、gRPC 服务端层
  `SignedContextLayer` 与客户端 `SignedContextInterceptor` / `set_signed_context_token`。
  令牌放在 `x-flare-ctx`，固定 `iss` 与 `aud` 为 `flare-internal`、
  `token_use = internal_context`，有效期 60 秒，携带 actor 类型与 ID、租户、角色、
  可选凭据 ID、request ID 与 jti。服务端剥离裸 `x-actor-*`、`x-user-id`、`x-tenant-id`
  等身份头，已验证的上下文不会再被元数据回填覆盖；未迁移的服务仍走原 `ContextLayer`。
  见 `docs/signed-context.md`。（bf15224）
- 动态负载均衡通道：`ServiceDiscover::into_balance_channel(fallback, request_timeout)`
  返回一个可克隆的 `discovery::BalancedChannel`（可直接作为 tonic 生成客户端的传输
  层），其节点集合随服务发现的增删刷新，不再固定在首次选中的节点；静态回落地址只在
  没有已注册节点时使用；所有克隆释放后转发任务随之退出。（c143f19）
  它不用 tonic 自带的 `Channel::balance_channel`：后者的懒连接在建连失败后仍报就绪，
  服务发现发出 Remove 之前约有两成到一半的请求落到已停的副本上。`BalancedChannel`
  在连接级失败后隔离该节点、按退避重新建连探测，成功后自动恢复（同一实例原地重启
  不需要重新发现）；P2C 以在途请求数为负载，避开无响应的副本；节点连接带建连超时与
  HTTP/2 keepalive。没有可用节点时请求会排队，调用方须自带端到端截止时间。
- JSON 日志：`LoggingSubscriberOptions` 增加 `format`（`LogFormat::Text` /
  `LogFormat::Json`）。JSON 每个事件一行，字段平铺并带当前 span，不含 ANSI 转义，
  可与 OTLP trace 导出同时启用；未知格式名解析为 `UnknownLogFormat` 错误，而不是
  悄悄回落到文本。（66c303b）
- `shutdown_tracing(timeout)`：退出前关闭 OTLP exporter 并导出批处理队列里剩下的
  span，未启用 OTLP 时为空操作。批处理默认 5 秒一批，进程退出前不调用会丢掉最后一批；
  须在 tokio 运行时仍存活时调用。（273ecc5）
- `TokenService::generate_token_with_use` / `validate_token_with_use` 与 `TOKEN_USE_CONTROL`:
  带显式用途的短期服务间令牌(控制面 → 数据面),接收方校验 `token_use`,普通接入令牌
  打不了管理接口。`generate_token_with_ttl` 改为其特例,行为不变。
- `TOKEN_USE_ADMIN = "admin"`:`AuthenticatedPrincipal::from_token_claims` 对该用途派生 `admin:*` scope,
  让核 admin-gateway 能接受本地 HS256 签发的管理令牌(此前 scopes 恒空,任何令牌都 403)。

### 修复

- 服务发现的初始快照超过内部队列容量（16）时构造会阻塞：初始实例改为先缓存、
  由消费方读取，单次初始快照上限 4096 个实例；发现器销毁时取消挂起的定时刷新并释放
  后端。（c143f19）
- NATS 连接使用 URL 内嵌的凭据：`async-nats` 能解析 `nats://user:pass@host`，
  却不会在建连时发送其中的凭据，要求鉴权的 broker 因此拒绝所有连接。生产者与消费者
  改走统一建连，取出用户名口令或令牌（百分号解码）作为连接选项传入，错误信息不含
  凭据。（ec833a0）
- 令牌撤销只认显式撤销位：切换到 v2 键时曾把“不在 v2 活跃集里”当作已撤销，
  而外部可信签发方的令牌（如 Social 自签的 IM 连接令牌）从不在 Core 令牌库登记，
  接入网关因此拒绝所有这类连接。现在只有 v2 或切换前的撤销位存在才算撤销，切换前
  已撤销的令牌在到期前仍被拒绝。此问题由本版本的 bf15224 引入，未进入任何已发布
  版本。（a45549e）
- 签名上下文对 `tenant_admin` 声明也以 actor ID 还原 `user_id`：此前只有 `user`
  声明会设置，而签发端签租户管理员时以管理员本人 ID 同时作为操作者，需要操作者的
  处理器（如控制面的群治理、冻结账号）因此一律拒绝。`service`、`system`、`guest`
  仍没有 `user_id`，管理类放行仍只看签名里的角色。此问题由本版本的 bf15224 引入，
  未进入任何已发布版本。（99d00e5）

### 其他

- 补齐 crate 的发布元信息（关键词、分类等）。没有关键词等于在 crates.io
  上搜不到——对一个希望被采用的开源包，这和功能缺失一样实际。

## 1.1.0 — 2026-08-03

与契约层 `flare-proto` 2.0.1 对齐的实现层发布。

## 1.0.3 — 2026-07-05

见 git tag `1.0.3`。
