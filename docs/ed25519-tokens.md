# Ed25519 令牌信任根

`TokenService` 的自定义 JWT provider 同时支持现有 HS256/384/512 和 EdDSA。没有启用 aws-lc 后端。

- 签发端：`TokenService::from_ed25519_private_key(material, issuer, ttl_seconds)`。
- 校验端：`TokenService::from_ed25519_public_key(material, issuer)`，只持公钥，任何签发调用均失败。
- `generate_with_ed25519(subject, tenant_id, token_use, audience)` 使用服务 TTL，保存真实 actor subject 与 tenant。
- `validate_ed25519_with_use(token, expected_use, audience)` 固定 EdDSA，严格匹配 issuer/audience/use，检查 exp/iat/jti/sub，过期无额外宽限；已配置 TokenStore 时同时检查撤销。
- 密钥接受 Ed25519 PKCS#8 私钥 PEM、公钥 SPKI PEM，或其 base64 PEM/DER 表示；错误不包含密钥正文。
- HS256 的现有签发、刷新接口保持原行为。Ed25519 实例不能使用隐含用途/audience 的旧签发入口。

P0 控制令牌和内部上下文分别使用独立密钥对。本提交提供密码学基础；调用链鉴权、角色传递、启动时密钥注入在 SEC-01/02 接入。仅增加 provider 不代表控制 RPC 已受到保护。

验证：`cargo test -p flare-core-infra auth::token::tests`、`cargo test --workspace`。
