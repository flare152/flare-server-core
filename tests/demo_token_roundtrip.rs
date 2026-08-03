//! demo token 的签发 → 验签闭环。
//!
//! 开源部分不含账号体系，新手靠 `examples/mint_token.rs` 手签一个 token 把栈跑起来。
//! 如果签出来的 token 过不了服务端验签，那份「五分钟跑通」的承诺就是空的 ——
//! 而这恰恰是开源引流的第一道门，断在这里的人不会再回来。
//!
//! 所以这里钉死闭环：mint_token 用的那套参数签出的 token，
//! 必须能被网关实际使用的 `CoreJwtTokenValidator` 验过。

use flare_server_core::TokenService;
use flare_server_core::auth::CompositeTokenValidator;
use std::sync::Arc;

/// 与 examples/mint_token.rs 保持一致。改这里的话那边也要改。
const DEMO_SECRET: &str = "flare-local-demo-secret-do-not-use-in-production";
const DEMO_ISSUER: &str = "flare-demo";

fn demo_service() -> TokenService {
    TokenService::new(DEMO_SECRET, DEMO_ISSUER, 3600)
}

#[test]
fn minted_demo_token_validates_with_same_secret() {
    let svc = demo_service();
    let token = svc
        .generate_token("alice", None, Some("0"))
        .expect("签发应当成功");

    let claims = svc.validate_token(&token).expect("同密钥必须验签通过");
    assert_eq!(claims.sub, "alice");
    assert_eq!(claims.tenant_id.as_deref(), Some("0"));
}

#[test]
fn minted_token_carries_tenant_and_device_claims() {
    let svc = demo_service();
    let token = svc
        .generate_token("bob", Some("device-42"), Some("t9"))
        .expect("签发应当成功");

    let claims = svc.validate_token(&token).expect("验签应通过");
    assert_eq!(claims.sub, "bob");
    assert_eq!(claims.device_id.as_deref(), Some("device-42"));
    assert_eq!(
        claims.tenant_id.as_deref(),
        Some("t9"),
        "租户声明必须随 token 带出，否则下游按空租户处理"
    );
}

#[test]
fn token_signed_with_other_secret_is_rejected() {
    // 密钥不一致必须验签失败 —— 否则任何人都能伪造身份。
    let minted = TokenService::new("some-other-secret", DEMO_ISSUER, 3600)
        .generate_token("mallory", None, Some("0"))
        .expect("签发应当成功");

    assert!(
        demo_service().validate_token(&minted).is_err(),
        "异密钥签出的 token 必须被拒绝"
    );
}

#[test]
fn demo_token_passes_the_validator_the_gateway_actually_uses() {
    // 端到端的那一环：网关持有的是 Arc<dyn TokenValidator>，
    // 本地验签实现是 CoreJwtTokenValidator。上面几条只验了 TokenService 自身，
    // 这条确认 mint 出来的 token 能过网关真正使用的那条路径。
    let composite = CompositeTokenValidator::new(Arc::new(demo_service()));

    let token = demo_service()
        .generate_token("carol", None, Some("0"))
        .expect("签发应当成功");

    let claims = composite
        .validate_token(&token)
        .expect("网关使用的校验器必须能验过 demo token");
    assert_eq!(claims.sub, "carol");
}

#[test]
fn token_expiry_claim_reflects_configured_ttl() {
    // 校验过期机制本身：exp 必须按 ttl 设置，且晚于 iat。
    //
    // 这里不用「签个 1 秒 token 然后 sleep」来测过期 —— `jsonwebtoken` 默认带
    // 60 秒时钟偏移宽限（leeway），本项目没有覆盖该默认值，所以刚过期的 token
    // 在 60 秒内仍会被接受。那是容忍集群时钟漂移的正确行为，不是缺陷；
    // 但也意味着想真测到拒绝就得硬等 60 秒以上，不值得放进 CI。
    let ttl = 3600u64;
    let svc = TokenService::new(DEMO_SECRET, DEMO_ISSUER, ttl);
    let token = svc.generate_token("dave", None, Some("0")).unwrap();

    let claims = svc.validate_token(&token).expect("验签应通过");
    let span = claims.exp.saturating_sub(claims.iat) as u64;
    assert_eq!(span, ttl, "exp - iat 必须等于配置的 ttl");
}
