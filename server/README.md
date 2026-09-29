# mistake-agent-server

Mistake Agent 服务端（ADR-0047/0048/0049）：账号体系、兑换码售卖、DeepSeek 中转、多设备同步。
**独立部署单元**：与仓库根 crate（`mistake-agent`）没有 Cargo 关系，不进客户端二进制。

> 当前进度：**S2（账号与鉴权）**。已落地账号、令牌、角色守卫与管理面账号查询；
> `relay`（S3 中转）、`billing`（S4 套餐与兑换码）、`sync`（S6）待做，见 [docs/TODO.md](../docs/TODO.md) 的 S1–S8。

## 快速开始（开发）

```bash
# 1. 起数据库（PostgreSQL 16，仅绑定回环）
docker compose up -d

# 2. 配置
cp .env.example .env      # 默认值直接可用

# 3. 运行（启动时自动应用 migrations/；配了 ADMIN_EMAIL/ADMIN_PASSWORD 会幂等创建管理员）
cargo run
```

验证：

```bash
curl -s http://127.0.0.1:8080/healthz   # {"status":"ok",...}  —— 存活，不碰数据库
curl -s http://127.0.0.1:8080/readyz    # {"status":"ready",...} —— 就绪，含数据库探活
```

## 账号 API

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| POST | `/api/v1/auth/register` | — | 自助注册（固定为 `user` 角色） |
| POST | `/api/v1/auth/login` | — | 登录，返回令牌（**明文只此一次**） |
| POST | `/api/v1/auth/logout` | Bearer | 撤销当前令牌（不影响其它设备） |
| GET | `/api/v1/me` | Bearer | 账号状态：邮箱 / 角色 / `sync_enabled` |
| PATCH | `/api/v1/me` | Bearer | 局部更新展示名与同步开关 |
| GET | `/api/v1/admin/users` | Bearer + admin | 分页列出账号（`?limit=&offset=`） |

```bash
EMAIL="a@example.test"
curl -s -X POST http://127.0.0.1:8080/api/v1/auth/register \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"secret-password\"}"

TOKEN=$(curl -s -X POST http://127.0.0.1:8080/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"secret-password\"}" | sed 's/.*"token":"\([^"]*\)".*/\1/')

curl -s http://127.0.0.1:8080/api/v1/me -H "Authorization: Bearer $TOKEN"
```

错误体统一为 `{"error":{"code":"...","message":"..."}}`；客户端按**状态码**分流
（401 重新登录、403 停用或越权、409 邮箱已注册、400 参数校验失败）。

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
| `DEEPSEEK_API_KEY` | S3 起 | 空 | 平台密钥（S1/S2 允许为空，仅告警） |
| `DEEPSEEK_BASE_URL` | | `https://api.deepseek.com` | 上游地址 |

## 端点

| 路径 | 说明 |
|---|---|
| `GET /healthz` | 存活探针，**不碰数据库** |
| `GET /readyz` | 就绪探针，探数据库；不可达返回 503 |
| `/api/v1/*` | 自有业务面：账号（S2）、套餐（S4）、同步（S6） |
| `POST /responses` | DeepSeek 兼容中转（S3） |

未知路径统一返回 `404` + `{"error":{"message":"..."}}`。

## 测试

```bash
cargo test                       # 单元 + 集成（集成需可写的 PostgreSQL）
cargo clippy -- -D warnings
cargo fmt --check
```

- **不依赖数据库的测试**（`tests/health.rs`）：懒连接池构造路由，任何环境都能跑。
- **依赖数据库的测试**（`tests/auth.rs`）：连接串取 `TEST_DATABASE_URL` → `DATABASE_URL` → `server/.env`；
  三者都没有时给出明确失败信息。测试用随机邮箱隔离，**不清库**（避免误删开发数据）。
  CI 里由 job 内的 `postgres` service 提供（见 `.github/workflows/ci.yml`）。

## 目录

```
server/
├── migrations/          sqlx 迁移（编译期嵌入，部署单二进制不依赖工作目录）
├── src/
│   ├── lib.rs           模块出口（便于集成测试）
│   ├── main.rs          进程入口：配置 → 日志 → 数据库 → 迁移 → 管理员种子 → HTTP
│   ├── config.rs        环境变量配置与校验
│   ├── logging.rs       分级日志 + 连接串脱敏
│   ├── db.rs            连接池与迁移
│   ├── auth/            账号、令牌与鉴权
│   │   ├── mod.rs         公共面与路由装配
│   │   ├── model.rs       领域类型（User/Role/AuthUser）与输入校验
│   │   ├── password.rs    Argon2id 哈希与校验
│   │   ├── token.rs       不透明令牌签发与 SHA-256 摘要
│   │   ├── store.rs       用户/令牌的数据库读写
│   │   ├── error.rs       错误 → HTTP 呈现
│   │   ├── extract.rs     AuthUser / RequireAdmin 提取器
│   │   └── handlers.rs    HTTP 处理函数与 DTO
│   ├── admin/           管理面（账号查询；S4 起加兑换码发放）
│   └── http/            路由装配与基础设施端点
├── tests/               集成测试（common/ 为共享夹具）
└── docker-compose.yml   开发期 PostgreSQL
```

后续按 [ADR-0047](../docs/adr/0047-server-account-package-relay.md) 增补：
`relay/`（S3 中转）、`billing/`（S4 套餐与兑换码）、`sync/`（S6）、admin CLI（S4）。
部署运维手册在 S8 产出（`docs/server.md`）。
