//! 签发一个 demo 用的接入 token —— **不需要任何用户体系**。
//!
//! 开源部分是通信基础设施，不含账号体系（注册登录在商业部分）。但网关的验签
//! 是可插拔契约：`CoreJwtTokenValidator` 用共享密钥本地验 JWT，所以只要手里有一个
//! 用同一密钥签出来的 token，就能把整套开源栈跑起来做 demo / POC。
//!
//! 这个工具就是干这个的。
//!
//! ```bash
//! # 密钥必须与服务端一致 —— start_server.sh 生成的那把在 logs/.dev-token-secret
//! export FLARE_TOKEN_SECRET="$(cat ../flare-im-core/logs/.dev-token-secret)"
//! cargo run --example mint_token -- alice
//!
//! # 指定租户与有效期
//! cargo run --example mint_token -- alice --tenant 0 --ttl 86400
//!
//! # 用你自己的密钥（必须与服务端配置的一致）
//! FLARE_TOKEN_SECRET=你的密钥 cargo run --example mint_token -- alice
//! ```
//!
//! 拿到 token 后：
//!
//! ```bash
//! curl -H "Authorization: Bearer <token>" http://127.0.0.1:50050/api/v1/...
//! ```
//!
//! 注意：**默认密钥仅供本地 demo。** 生产环境必须通过 `FLARE_TOKEN_SECRET` 注入
//! 强密钥，且与服务端配置一致 —— 弱密钥意味着任何人都能伪造任意用户的 token。

use flare_server_core::TokenService;

/// 服务端没有内置默认密钥 —— `scripts/start_server.sh` 会随机生成一把存进
/// `flare-im-core/logs/.dev-token-secret`，并注入给各网关。所以本工具**必须**
/// 用同一把密钥签发，否则验签必失败（表现为接口 401）。
///
/// 这里保留一个兜底常量只是为了在未提供密钥时给出明确报错，而不是静默签出
/// 一个注定用不了的 token。
const NO_SECRET_HINT: &str = "\
未提供签名密钥。服务端密钥由 start_server.sh 随机生成，请这样取用：

    cd flare-im-core
    export FLARE_TOKEN_SECRET=\"$(cat logs/.dev-token-secret)\"

然后重新运行本工具。若你自行指定过 FLARE_API_GATEWAY_TOKEN_SECRET，
请改用那把密钥。";
const DEFAULT_ISSUER: &str = "flare-im-core";
const DEFAULT_TTL_SECS: u64 = 24 * 3600;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut user_id: Option<String> = None;
    let mut tenant_id = "0".to_string();
    let mut ttl = DEFAULT_TTL_SECS;
    let mut device_id: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "--tenant" => {
                i += 1;
                match args.get(i) {
                    Some(v) => tenant_id = v.clone(),
                    None => return fail("--tenant 需要一个值"),
                }
            }
            "--device" => {
                i += 1;
                match args.get(i) {
                    Some(v) => device_id = Some(v.clone()),
                    None => return fail("--device 需要一个值"),
                }
            }
            "--ttl" => {
                i += 1;
                match args.get(i).map(|v| v.parse::<u64>()) {
                    Some(Ok(v)) if v > 0 => ttl = v,
                    _ => return fail("--ttl 需要一个正整数（秒）"),
                }
            }
            other if other.starts_with('-') => {
                return fail(&format!("未知参数：{other}"));
            }
            other => user_id = Some(other.to_string()),
        }
        i += 1;
    }

    let Some(user_id) = user_id else {
        print_usage();
        std::process::exit(2);
    };

    let secret = match std::env::var("FLARE_TOKEN_SECRET") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => {
            eprintln!("{NO_SECRET_HINT}");
            std::process::exit(2);
        }
    };
    let issuer = std::env::var("FLARE_TOKEN_ISSUER").unwrap_or_else(|_| DEFAULT_ISSUER.to_string());

    let service = TokenService::new(secret, issuer, ttl);
    match service.generate_token(&user_id, device_id.as_deref(), Some(&tenant_id)) {
        Ok(token) => {
            // token 本身走 stdout，方便 $(...) 直接取用；
            // 说明性文字一律走 stderr，不污染管道。
            eprintln!("user_id={user_id} tenant_id={tenant_id} ttl={ttl}s");
            println!("{token}");
        }
        Err(e) => fail(&format!("签发失败：{e}")),
    }
}

fn print_usage() {
    eprintln!(
        r#"签发 demo 接入 token（无需用户体系）

用法:
    cargo run --example mint_token -- <user_id> [选项]

选项:
    --tenant <id>   租户 ID（默认 "0"）
    --device <id>   设备 ID（可选）
    --ttl <秒>      有效期（默认 86400）
    -h, --help      显示本帮助

环境变量:
    FLARE_TOKEN_SECRET   签名密钥（**必填**），必须与服务端一致。
                         start_server.sh 生成的那把在
                         flare-im-core/logs/.dev-token-secret
    FLARE_TOKEN_ISSUER   签发者（默认 "flare-im-core"，与网关配置一致）

示例:
    export FLARE_TOKEN_SECRET="$(cat ../flare-im-core/logs/.dev-token-secret)"
    TOKEN=$(cargo run -q --example mint_token -- alice)
    curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:50050/api/v1/..."#
    );
}

fn fail(msg: &str) {
    eprintln!("错误：{msg}\n");
    print_usage();
    std::process::exit(2);
}
