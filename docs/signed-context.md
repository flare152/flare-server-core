# Cell 内部签名上下文

`SignedContextLayer` 是需要可信 actor/tenant 的 gRPC 服务入口。签发端持
`FLARE_INTERNAL_CTX_SIGNING_KEY`（Ed25519 PKCS#8 私钥），校验端只持
`FLARE_INTERNAL_CTX_PUBLIC_KEY`（SPKI 公钥）。密钥按 Cell 配置；不要向前端或仅校验的服务
分发私钥。

入站只从 `x-flare-ctx` 建立身份。令牌固定 `iss=flare-internal`、
`aud=flare-internal`、`token_use=internal_context`，最长 60 秒，携带 actor 类型、
actor ID、租户、角色、可选 credential ID、request ID 和唯一 jti。校验包括 EdDSA
签名、issuer/audience/use、必填字段、时间区间、类型及身份约束。服务端剥离裸
`x-actor-*`、`x-user-id`、`x-tenant-id` 等身份头。`require_ctx_from_request` 遇到
已验证的 Context 后不再从 metadata 回填用户、租户或设备；未迁移的服务仍使用原来的
`ContextLayer` 路径，两者不会因为同名导出而使用不同实现。

已验证 Context 的 `user_id` 只由声明决定：`user` 与 `tenant_admin` 的 actor ID 同时
作为 `user_id`（租户管理员的 actor ID 就是已认证管理员本人的 ID，处理器把它记为操作
者）；`service`、`system`、`guest` 不是用户，没有 `user_id`。这不带来额外权限，管理类
放行仍只看签名里的角色。签发端签 `tenant_admin` 时只能用已认证管理员本人的 ID，不得
代签别的用户。

```rust,ignore
use flare_server_core::{
    SignedActorType, SignedContextIdentity, SignedContextLayer,
    SignedContextSigner, SignedContextVerifier, set_signed_context_token,
};

let signer = SignedContextSigner::from_private_key(&signing_key)?;
let identity = SignedContextIdentity::new(
    SignedActorType::User, authenticated_user_id,
    validated_tenant_id, request_id,
);
let token = signer.sign(&identity)?;
set_signed_context_token(&mut request, &token)?;

let verifier = SignedContextVerifier::from_public_key(&public_key)?;
let service = SignedContextLayer::new(verifier).layer(grpc_service);
```

签发端必须先完成原始请求的用户/凭据、租户和权限验证，再构造
`SignedContextIdentity`；不可把裸 gRPC/HTTP metadata 编入令牌。默认缺失或错误令牌
会返回 `UNAUTHENTICATED`。仅明确公开的整项服务才调用 `allow_anonymous()`；该模式
仍拒绝无效令牌，并产生无租户、无用户的匿名 verified marker。需要公开与受保护方法
并存时，应由已验证租户的上游为公开方法签 Guest，然后在应用层按 RPC 方法授权。

已验签入站 Context 中的 `VerifiedContext::token()` 可供下游原样转发；客户端
`SignedContextInterceptor` 每次 RPC 调用 token provider，不缓存短期令牌。
同一令牌在 60 秒内允许转发和重试，所以通用校验层不把 jti 当成一次性消费标记。
对有副作用的命令，应用层仍需用 jti 或明确的幂等键处理重复调用；传输层私网/TLS
与最小角色授权仍是这段重放窗口的安全边界。

## 迁移中的服务：签名优先（`SignedFirstContextLayer`）

还有调用方只发裸 `x-tenant-id` / `x-user-id` 的服务（例如 Core 的会话读、Storage Reader 与
同步），用 `SignedFirstContextLayer` 代替 `ContextLayer`：

- 带一个 `x-flare-ctx` 时验签，身份只取自声明；同时带来的裸 `x-tenant-id`、`x-user-id`、
  `x-actor-id`、`x-actor-type`、`x-actor-roles` 必须与声明一致（`service`、`system` 没有用户，
  带 `x-user-id` 即不一致），否则 `PERMISSION_DENIED`。通过后与 `SignedContextLayer` 一样剥离
  裸身份头，`VerifiedContext::token()` 可供下游转发。
- 默认只收 `user`、`tenant_admin`、`service` 三类 actor（`allow_actor_types` 可改），其余
  `PERMISSION_DENIED`；验签失败、重复或伪造的令牌 `UNAUTHENTICATED`。
- 不带令牌：`require_signed(true)` 时 `UNAUTHENTICATED`；否则沿用元数据身份，与
  `ContextLayer::allow_missing` 相同。
- 没有验证器（未配公钥）：要求签名时一律 `UNAVAILABLE`；不要求时令牌无法校验、不带来任何
  身份，仍按元数据，与未装这一层时相同。
- `exempt_method("/pkg.Service/Method")` 标出由处理器自行鉴权的方法（如只认转发的 admin
  令牌的管理检索），它们沿用元数据上下文，不要求签名。

调用方转发已验签的入站身份用 `forward_signed_context(&mut request, &ctx)`：只有经验签建立的
Context 才带令牌，元数据身份或匿名上下文不转发任何东西。与 `request_with_context` 配合时，
元数据即签名身份，接收方不会判为冲突；从已验证上下文派生出别的租户或用户再转发，接收方会拒绝。

## 密钥集与 kid（v4 R3-1）

签发端可以声明自己的密钥 id：`SignedContextSigner::from_private_key(..)?.with_kid("gateway")?`
把 `kid` 写进 JWT 头。校验端持一个 `KeySet`（kid → 公钥）：

- `FLARE_INTERNAL_CTX_PUBLIC_KEYS`：JSON 对象 `{"gateway": "<SPKI PEM 或 base64>", "hook": ...}`，
  或一个目录，目录里每个组件一个 `<kid>.pem`（或 `.pub`）文件。
- `FLARE_INTERNAL_CTX_PUBLIC_KEY`：单把 Cell 公钥，登记为 `kid=legacy`；没有 `kid` 头的令牌按
  它校验。
- 两个都设时组件集叠加在 legacy 之上；都不设时 `SignedContextVerifier::from_env()` 报错，
  `from_env_optional()` 回 `None`。

只有单钥部署（只设 `FLARE_INTERNAL_CTX_PUBLIC_KEY`）时，令牌里任何 kid 都回落到这把钥：
组件密钥由 flarectl 生成之前（R6-2）kid 钉死只是名义上的；R6-4 删除这条回落。只要集合里
出现任何一把组件钥，未登记的 kid 一律拒绝。

`require_staff_kid(["gateway"])` 把 `Staff` 上下文钉到指定组件：别的组件签的 `Staff` 一律
`UNAUTHENTICATED`，其余 actor 类型不受影响。Core 的管理面（admin-gateway 与它的四个下游）
只收网关签的 `Staff`；被攻破的 Social 服务拿不到 gateway 钥，伪造不了员工身份。
验签后的 `SignedContextClaims::kid` 与 actor 属性 `kid` 记录实际命中的组件。

`SignedContextIdentity::with_trace_id` 把发起请求的追踪号签进令牌（最长 128 字节），校验端
写进 `Context::trace_id`；它只用于关联日志，不是身份。

### Staff 权限门禁

Core 侧与 Social `flare-social-common::auth` 同一规则：

```rust,ignore
use flare_server_core::{require_staff_permission, require_staff_permission_any, staff_claims};
use flare_core_infra::auth::core_admin::permission;

let claims = require_staff_permission(&ctx, permission::USERS_KICK)?;
// 未验签上下文 UNAUTHENTICATED；非 Staff 或 permission 不符 PERMISSION_DENIED。
```

### Core 管理路由契约

`flare_core_infra::auth::core_admin` 是 Core 管理面的路由表：每条路由的方法、相对管理根的路径
模式、权限码（`core:conversations:read`、`core:users:kick`、`core:hooks:write` 等）、是否写、
是否仅平台员工。Social 网关 admin 分区用 `resolve(method, path)` 决定要签进 Staff 上下文的权限码，
Core admin-gateway 用同一张表校验收到的码；两端共用一份，不会漂移。

