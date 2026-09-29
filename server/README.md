# mistake-agent-server

Mistake Agent 服务端（ADR-0047/0048/0049）：账号体系、兑换码售卖、DeepSeek 中转、多设备同步。
**独立部署单元**：与仓库根 crate（`mistake-agent`）没有 Cargo 关系，不进客户端二进制。

> 当前进度：**S1（骨架）**。端点只有基础设施探针；`auth` / `relay` / `billing` / `sync` 待落地，见 [docs/TODO.md](../docs/TODO.md) 的 S1–S8。

## 快速开始（开发）

```bash
# 1. 起数据库（PostgreSQL 16，仅绑定回环）
docker compose up -d

# 2. 配置
cp .env.example .env      # 默认值直接可用

# 3. 运行（启动时自动应用 migrations/）
cargo run
```

验证：

```bash
curl -s http://127.0.0.1:8080/healthz   # {"status":"ok",...}  —— 存活，不碰数据库
curl -s http://127.0.0.1:8080/readyz    # {"status":"ready",...} —— 就绪，含数据库探活
```

质量门（与客户端各自独立）：

```bash
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

## 配置

配置来源唯一：**环境变量**（生产经 systemd `EnvironmentFile` 注入；开发期 `.env` 由 dotenvy 加载）。
启动时一次性读取并校验，缺失/非法即拒绝启动（fail-fast）。

| 变量 | 必需 | 默认 | 说明 |
|---|---|---|---|
| `DATABASE_URL` | ✅ | — | PostgreSQL 连接串（日志中已脱敏） |
| `BIND_ADDR` | | `127.0.0.1:8080` | 监听地址；TLS 由前置反向代理终结 |
| `DB_MAX_CONNECTIONS` | | `10` | 连接池上限 |
| `LOG_LEVEL` | | `info` | 分级日志；`RUST_LOG` 存在时优先 |
| `DEEPSEEK_API_KEY` | S3 起 | 空 | 平台密钥（S1 允许为空，仅告警） |
| `DEEPSEEK_BASE_URL` | | `https://api.deepseek.com` | 上游地址 |

## 端点

| 路径 | 说明 |
|---|---|
| `GET /healthz` | 存活探针，**不碰数据库** |
| `GET /readyz` | 就绪探针，探数据库；不可达返回 503 |
| `POST /responses` | DeepSeek 兼容中转（S3） |
| `/api/v1/*` | 自有业务面：账号（S2）、套餐（S4）、同步（S6） |

未知路径统一返回 `404` + `{"error":{"message":"..."}}`。

## 目录

```
server/
├── migrations/          sqlx 迁移（编译期嵌入，部署单二进制不依赖工作目录）
├── src/
│   ├── lib.rs           模块出口（便于集成测试）
│   ├── main.rs          进程入口：配置 → 日志 → 数据库 → 迁移 → HTTP
│   ├── config.rs        环境变量配置与校验
│   ├── logging.rs       分级日志 + 连接串脱敏
│   ├── db.rs            连接池与迁移
│   └── http/            路由装配（mod.rs）与基础设施端点（health.rs）
├── tests/health.rs      端点集成测试（无需真实数据库）
└── docker-compose.yml   开发期 PostgreSQL
```

后续按 [ADR-0047](../docs/adr/0047-server-account-package-relay.md) 增补：`auth/`（S2）、`relay/`（S3）、`billing/`（S4）、`sync/`（S6）、`admin/`（CLI）。
部署运维手册在 S8 产出（`docs/server.md`）。
