# mistake-agent-server

Mistake Agent 服务端（ADR-0047/0048/0049）：账号体系、兑换码售卖、DeepSeek 三协议中转、多设备同步。
**独立部署单元**：与仓库根 crate（`mistake-agent`）没有 Cargo 关系，不进客户端二进制。

> 当前进度：**S1–S3**。已落地骨架、账号与鉴权、**三类协议的中转网关（带配额与计费）**与安全护栏；
> `billing` 的兑换码与 admin CLI（S4）、`sync`（S6）、部署（S8）待做，见 [docs/TODO.md](../docs/TODO.md)。

## 快速开始（开发）

```bash
docker compose up -d      # PostgreSQL 16，仅绑定回环
cp .env.example .env      # 默认值直接可用
cargo run                 # 启动时自动应用 migrations/；配了 ADMIN_EMAIL/ADMIN_PASSWORD 会幂等建管理员
```

```bash
curl -s http://127.0.0.1:8080/healthz   # 存活，不碰数据库
curl -s http://127.0.0.1:8080/readyz    # 就绪，含数据库探活
```

## 端点

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| GET | `/healthz` | — | 存活探针，**不碰数据库** |
| GET | `/readyz` | — | 就绪探针，探数据库；不可达 503 |
| POST | `/api/v1/auth/register` | — | 自助注册（固定 `user` 角色） |
| POST | `/api/v1/auth/login` | — | 登录，返回令牌（**明文只此一次**） |
| POST | `/api/v1/auth/logout` | Bearer | 撤销当前令牌 |
| GET | `/api/v1/me` | Bearer | 账号状态（邮箱/角色/`sync_enabled`） |
| PATCH | `/api/v1/me` | Bearer | 局部更新展示名与同步开关 |
| GET | `/api/v1/admin/users` | Bearer + admin | 分页列出账号 |
| POST | `/responses`、`/v1/responses` | Bearer / `x-api-key` | **中转**：OpenAI Responses 面 |
| POST | `/chat/completions`、`/v1/chat/completions` | 同上 | **中转**：OpenAI Chat Completions 面 |
| POST | `/messages`、`/v1/messages` | 同上 | **中转**：Anthropic Messages 面 |

错误体统一为 `{"error":{"code":"...","message":"..."}}`；限流/封禁类响应带 `Retry-After`。

### 中转（S3）

带鉴权与计费的**透传网关**。三个协议面在上游（DeepSeek）都有同名端点，所以**没有任何协议翻译**——
差异只有三处，全部集中在 `src/relay/protocol.rs` 与 `src/relay/sse.rs`：上游路径映射、
usage 字段归一化、用户隔离字段名。

请求生命周期：

```
鉴权 → 令牌桶限流（按用户 + 按 IP）→ 并发闸门（按用户 + 全局）
     → 预扣（裁决 + 写 reserved 流水 + 权益 +1）
     → 转发上游（覆盖模型名、注入平台 user id）
     → 流式 tee（原样转发，旁路解析 usage 与终态）
     → 结算（按阶梯上调扣次；上游明确失败则退回）
```

| 结果 | 状态码 | 记账 |
|---|---|---|
| 正常结束且有 usage | 200 | `ok`，按阶梯扣 1/2/3 次 |
| 正常结束但无 usage（上游提前断流） | 200 | `aborted`，看到过事件就扣 1 次 |
| 上游明确失败（`response.failed` 等） | 200/4xx 透传 | `upstream_error`，不扣次 |
| 上游连不上 | 502 | `upstream_error`，不扣次 |
| 额度不足（含无权益、窗口超限） | 402 | 不进入上游 |
| 同一用户并发超限 | 429 | — |
| 限流（用户桶/IP 桶）或全局繁忙 | 429 + `Retry-After` | — |
| 非流式请求（`stream: false`） | 400 `stream_required` | — |

**只支持流式**：非流式无法可靠旁路取用量，计费就无从谈起。

### 安全护栏（S3）

三层，见 `src/security/`：

1. **令牌桶限流**：账号面按 IP、中转面按用户与按 IP。选令牌桶是因为它突发友好
   （学生一轮工具调用会连发数个请求）且状态 O(1)/key。
2. **失败封禁（fail2ban 语义）**：登录失败按 IP **与账号**双维度计数，默认 5 次 / 10 分钟 → 封 15 分钟；
   同时输出固定格式日志行供外部 fail2ban 长期封禁。配置与安装见 [deploy/fail2ban](deploy/fail2ban/)。
3. **全局并发上限**：令牌桶管速率，管不住"同时挂着一堆长流式请求"，因此另有全局计数器保护
   上游账号与进程容量。

## 配置

配置来源唯一：**环境变量**（生产经 systemd `EnvironmentFile` 注入；开发期 `.env` 由 dotenvy 加载）。
启动时一次性读取并校验，缺失/非法即拒绝启动（fail-fast）。

| 变量 | 必需 | 默认 | 说明 |
|---|---|---|---|
| `DATABASE_URL` | ✅ | — | PostgreSQL 连接串（日志中已脱敏） |
| `BIND_ADDR` | | `127.0.0.1:8080` | 监听地址；TLS 由前置反向代理终结 |
| `DB_MAX_CONNECTIONS` | | `10` | 连接池上限 |
| `LOG_LEVEL` | | `info` | 分级日志；`RUST_LOG` 存在时优先 |
| `TOKEN_TTL_DAYS` | | `90` | 登录令牌有效期（天） |
| `ADMIN_EMAIL` / `ADMIN_PASSWORD` | | 空 | 管理员种子：两者都配齐且邮箱未注册时创建，幂等；**不会**提升已存在的同邮箱账号 |
| `DEEPSEEK_API_KEY` | ✅（中转） | 空 | 平台密钥 |
| `DEEPSEEK_BASE_URL` | | `https://api.deepseek.com` | 上游地址 |
| `DEEPSEEK_MODEL` | | `deepseek-flash` | 中转强制使用的模型（不信任客户端传来的 model） |
| `BILLING_LADDER_TOKENS` | | `32768,65536` | 阶梯扣次的 token 阈值（严格递增；空 = 不设阶梯） |
| `RELAY_MAX_BODY_BYTES` | | `33554432` | 中转请求体上限（32 MiB，图片 base64 内联需要） |
| `RELAY_MAX_CONCURRENT_PER_USER` | | `2` | 同一用户同时在飞的中转请求上限 |
| `SECURITY_TRUST_PROXY` | | `false` | 是否信任 `X-Forwarded-For`/`X-Real-IP`。**只有在反向代理后面才该开** |
| `SECURITY_AUTH_RATE_PER_MINUTE` / `_BURST` | | `30` / `10` | 账号面令牌桶（按 IP） |
| `SECURITY_RELAY_RATE_PER_MINUTE` / `_BURST` | | `60` / `60` | 中转面令牌桶（按用户与按 IP） |
| `SECURITY_LOGIN_MAX_FAILURES` | | `5` | 登录失败阈值 |
| `SECURITY_LOGIN_FAILURE_WINDOW_SECS` | | `600` | 失败计数窗口 |
| `SECURITY_LOGIN_BLOCK_SECS` | | `900` | 封禁时长 |
| `SECURITY_MAX_CONCURRENT_GLOBAL` | | `200` | 全局同时在飞上限 |

## 测试

```bash
cargo test                       # 单元 + 集成（集成需可写的 PostgreSQL）
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

- **不依赖数据库**（`tests/health.rs`）：懒连接池构造路由，任何环境都能跑。
- **依赖数据库**（`tests/auth.rs`）：连接串取 `TEST_DATABASE_URL` → `DATABASE_URL` → `server/.env`；
  测试用随机邮箱隔离，**不清库**。
- **流式解析**（`src/relay/sse.rs` 的单测）：直接喂 `tests/fixtures/` 下三个协议的**真实抓包**
  （2026-09-29 捕获、无密钥残留），把三面 usage 落点与容错行为钉死。

## 目录

```
server/
├── migrations/             sqlx 迁移（编译期嵌入；0001 账号、0002 计费）
├── deploy/fail2ban/        fail2ban filter/jail（进程内封禁的长期化补充）
├── src/
│   ├── main.rs             进程入口：配置 → 日志 → 数据库 → 迁移 → 管理员种子 → HTTP
│   ├── config.rs           环境变量配置与校验
│   ├── logging.rs          分级日志 + 连接串脱敏
│   ├── db.rs               连接池与迁移
│   ├── auth/               账号、令牌与鉴权（model/password/token/store/error/extract/handlers）
│   ├── admin/              管理面（账号查询；S4 起加兑换码发放）
│   ├── billing/            套餐、权益、用量与限额裁决（model/ladder/quota/store）
│   ├── relay/              三协议透传网关（protocol/sse/upstream/concurrency/error/handlers）
│   ├── security/           限流（令牌桶）、失败封禁、全局并发（ratelimit/lockout/client_ip）
│   └── http/               路由装配与基础设施端点
├── tests/                  集成测试（common/ 为共享夹具，fixtures/ 为真实抓包）
└── docker-compose.yml      开发期 PostgreSQL
```

## 两个环境相关的坑（会影响构建，不是代码问题）

1. **TLS 栈刻意避开 rustls**：`ring` 与 `aws-lc-sys` 都带 C 代码，在**含非 ASCII 字符的构建路径**
   （例如本机的 `...\项目代码\Web与应用\...`）下 `cc`/`cl.exe` 会编译失败。
   服务端因此用 `native-tls`（Windows = schannel，Linux = OpenSSL），不触发 C 编译。
   Linux CI 需要 `libssl-dev`（已在 CI job 里安装）。
   客户端（根 crate）同期依赖 `ring`/`aws-lc-sys`，目前靠旧的构建缓存才能编译——
   `cargo clean` 后会暴露同样的问题，需要单独修。
2. **转换头仅在可信代理后信任**：默认 `SECURITY_TRUST_PROXY=false`，
   否则任何人都能伪造来源 IP 绕过按 IP 的限流与封禁。

后续按 [ADR-0047](../docs/adr/0047-server-account-package-relay.md) 增补：兑换码与 admin CLI（S4）、
`sync/`（S6）。部署运维手册在 S8 产出（`docs/server.md`）。
