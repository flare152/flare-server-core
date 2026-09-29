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
