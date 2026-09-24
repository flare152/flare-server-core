# 变更记录

本文件只记录**已发布到 crates.io 的版本**。版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/)。

## 1.1.1 — 未发布

安全修复版本。1.1.0 发布于 2026-08-03，此后修掉的公告都在这里。
**建议所有 1.1.0 用户升级**。

### 安全

- `async-nats` 0.42 → 0.50，消除 `rustls-webpki` 0.102 的**四条公告**。
  依赖是间接引入的，升级 `async-nats` 是唯一能把它带上去的路径。

### 新增

- `TokenService::generate_token_with_use` / `validate_token_with_use` 与 `TOKEN_USE_CONTROL`:
  带显式用途的短期服务间令牌(控制面 → 数据面),接收方校验 `token_use`,普通接入令牌
  打不了管理接口。`generate_token_with_ttl` 改为其特例,行为不变。
- `TOKEN_USE_ADMIN = "admin"`:`AuthenticatedPrincipal::from_token_claims` 对该用途派生 `admin:*` scope,
  让核 admin-gateway 能接受本地 HS256 签发的管理令牌(此前 scopes 恒空,任何令牌都 403)。

### 其他

- 补齐 crate 的发布元信息（关键词、分类等）。没有关键词等于在 crates.io
  上搜不到——对一个希望被采用的开源包，这和功能缺失一样实际。

## 1.1.0 — 2026-08-03

与契约层 `flare-proto` 2.0.1 对齐的实现层发布。

## 1.0.3 — 2026-07-05

见 git tag `1.0.3`。
