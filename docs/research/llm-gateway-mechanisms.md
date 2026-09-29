# 中转网关实现机制研究（new-api / sub2api）

日期：2026-09-29
研究目的：为 mistake-agent 自研的「带鉴权 + 售卖服务包 + 多协议兼容」中转网关（ADR-0047）借鉴成熟项目的**工程机制**。
结论用途：S3 的实现取舍（哪些机制抄、哪些是渠道平台的重型设施不必抄）。

## 研究对象与方法

用 `git clone --depth 1 --single-branch` 浅克隆到临时目录后**读源码**（GitHub 页面在本环境取不到：`web_fetch` 报 hostname 解析到非公网 IP），读完即删除克隆，未复制任何代码进本仓库。

| 项目 | 版本（浅克隆时的 HEAD） | 许可证 | 定位 |
|---|---|---|---|
| [QuantumNous/new-api](https://github.com/QuantumNous/new-api) | `789c970199ea527e6a26e071915f4a4cd2c64178` | **AGPL-3.0** | 40+ 上游渠道的多渠道网关，含用户体系、计费、限流、管理台 |
| [changkaizhao/sub2api](https://github.com/changkaizhao/sub2api) | `0d27f45ead1b58908548ec21afd923ecaf7339bc` | **LGPL-3.0** | AI API 网关平台 ——「订阅配额分发」，把订阅账号额度分给拼车用户 |

> 说明：浅克隆时 new-api 仓库自带的 `AGENTS.md` / `CLAUDE.md` 被工具当作工作区指令载入过。那是**第三方仓库的内部协作规则**，与 mistake-agent 无关，未据此执行任何动作；这里只把它当作研究线索（它指向的 `service/tiered_settle.go`、`service/responses_usage.go`、`.agents/rules/billing.md` 确实是本研究最有价值的三个文件）。

## 一、许可证边界（先定这条，决定能抄什么）

- **new-api 是 AGPL-3.0**：网络服务也算分发，传染性最强。**不能抄代码**。
- **sub2api 是 LGPL-3.0**：以库形式链接有额外义务，直接搬代码同样有风险。
- 结论：**只借鉴机制与思路，不复制代码**。若将来确实要搬某段实现，必须先核对许可证并在文档注明来源（AGENTS.md 已要求）。本报告全部结论均为「读源码后用自己的话重述机制」。

## 二、new-api：分层与协议面

**分层**（`AGENTS.md` 的架构节 + 目录观察）：

- `router/` 路由、`middleware/` 中间件、`controller/` 入口、`service/` 业务与计费、`model/` GORM 模型、`relay/` 上游适配。
- **`relaykit/` 是独立的 Go module，只放协议 DTO 与格式转换**；传输、鉴权、数据库、计费**留在宿主**。
  → 这正是我们「网关闭环 / 协议适配 / 上游转发」三层划分的成熟对应物：**协议转换被隔成可独立演进的模块，计费与鉴权不跟着协议走**。

**外部协议面**（`router/relay-router.go`）：

```
/v1/chat/completions   /v1/completions   /v1/responses（另有 GET 走 WebSocket）
/v1/messages（Anthropic 格式）           /v1/embeddings  /v1/rerank  /v1/audio/*
```

中间件链：`RouteTag("relay")` → `SystemPerformanceCheck()` → `TokenAuth()` → `ModelRequestRateLimit()`。

**鉴权头嗅探**（`router/relay-router.go` L27/L38）：同时存在 `x-api-key` 且带 `anthropic-version` 时判定为 Anthropic 风格请求。
→ 我们采纳同样的兼容策略：中转路由同时接受 `Authorization: Bearer` 与 `x-api-key`。

## 三、new-api：令牌模型（`model/token.go` 的 `Token` 结构）

字段：`key`（唯一索引）、`status`、`name`、`created_time`、`accessed_time`、`expired_time`（**-1 = 永不过期**）、`remain_quota`、`unlimited_quota`、`model_limits_enabled` + `model_limits`、`allow_ips`、`used_quota`、`group`、`cross_group_retry`、`auto_groups`、`deleted_at`（软删）。

对照我们的 `tokens` 表（S2 已落地：`token_hash` / `label` / `expires_at` / `last_used_at` / `revoked_at`）：

| 他们的字段 | 我们的现状 | 取舍 |
|---|---|---|
| `model_limits`（令牌级模型白名单） | 无 | **值得补**：防止低档套餐拿便宜令牌跑贵模型；我们的套餐映射模型后天然需要它 |
| `deleted_at` 软删 | 硬删（`revoked_at` 只标撤销） | **值得补**：撤销后保留行便于审计追查 |
| `unlimited_quota` + `remain_quota`（令牌级额度） | 无（额度挂在账号的权益上） | 暂不采纳：我们的额度按「套餐权益」计，令牌级额度会引入第二套账 |
| `allow_ips` / `group` / `auto_groups` / `cross_group_retry` | 无 | 不采纳：多渠道平台的分组与跨组重试，单上游用不上 |

## 四、new-api：计费链路（最值得借鉴的部分）

**预扣 → 结算**（`service/billing.go`、`service/billing_session.go`、`service/quota.go`）：

```
PreConsumeBilling(c, preConsumedQuota, relayInfo)          service/billing.go
  └─ NewBillingSession(...)                                service/billing_session.go
       └─ if userQuota - preConsumedQuota < 0 → "预扣费额度失败"（不发起上游请求）
  转发上游
PostConsumeQuota(relayInfo, quota, preConsumedQuota, ...)  service/quota.go
  └─ postConsumeQuotaWithResult(...)  差額结算 + 额度告警
```

`BillingSession` 的关键状态：`preConsumedQuota`、`settled`、`refunded`，差额 `delta := actualQuota - preConsumedQuota`，**并且 `s.preConsumedQuota += delta` 允许预扣上调**（`service/tiered_settle_test.go` 里就是以 `preConsumedQuota: 50_000` 被上调到 `100_000` 来断言的）——即「先按估值扣，拿到真实用量后按差额补扣或退还」。另有 `FinalPreConsumedQuota` 落进 relay 信息供日志与对账使用。

**对我们的直接映射**（我们按「次数」卖，估值天然保守）：

- 请求开始时就**预留**（写一条 `status='reserved'`、`billed_uses=1` 的用量记录），窗口聚合把它算进去 → 并发下不会两个请求同时挤过窗口边界；
- 拿到 usage 后**结算**：UPDATE 该行为 `status='ok'` 与最终 `billed_uses`（阶梯可能把它从 1 上调到 2 或 3）；上游失败则改为 `upstream_error` 且 `billed_uses=0`（相当于退款）。

**计费安全不变量**（`.agents/rules/billing.md`，本报告最有价值的一节，逐条都值得抄进我们的测试）：

1. **绝不产生负扣费**（算术溢出或未校验输入导致的"退款"）。所有会成为计费乘数的**用户可控量**必须在请求校验阶段设上界并 400 拒绝。
2. **无符号类型的陷阱**：`*uint` 字段能接受 `18446744073686646784` 这种"回绕的负数"，所以 `>= 0` 的检查不充分，**上界是强制的**。
   （Rust 侧 serde 对负数反序列化成 `u64` 会直接报错，先天好一些；但上界仍然需要。）
3. **配额换算集中在一处**，禁止裸强转（`int(float64(q) * ratio)`），统一用 `common/quota_math.go` 的 `QuotaFromFloat` / `QuotaRound` / `QuotaFromDecimal`。
4. **饱和/clamp 事件必须可审计**：有 `*Checked` 变体返回 clamp 标记，计费路径把它挂到日志的 `admin_info.quota_saturation` 并打一条 warn，使异常计费可事后追查。挂在 `admin_info` 下的巧思：普通用户的日志视图会自动剥掉该字段。
5. **新增计费路径必须走完整链路核对**：校验 → 估算 → 换算 → 预扣 → 结算/退款，逐步确认不变量。

## 五、new-api：Responses 流式计费（照着我们 S3 的需求写的）

`service/responses_usage.go` 有一个专用的 `ResponsesUsageAccumulator`：

- **HTTP SSE 与 WebSocket 两种传输喂同一个累加器**，然后走统一的文本计费路径，**并且覆盖被中断的流**。
- **终态事件是一个集合**：`response.completed` / `response.done` / `response.failed` / `response.incomplete` / `response.cancelled` / `response.canceled`。
  → 我们的 tee 解析器不能只认 `response.completed`（这与我先前的设计相比是明确改进；官方文档只列了三个，实测与工程实践还要算上 done/cancelled 变体）。
- **所有 delta 类事件都算上游计费的输出**：`response.output_text.delta`、`response.function_call_arguments.delta`、`response.reasoning_summary_text.delta`、`response.reasoning_text.delta`、`response.refusal.delta` —— 累加它们的文本，用于**上游没给 usage 时的估算**。
- **中断流的计费规则**（关键）：`Finish()` 里若用量为空，用累计输出文本估算 completion tokens；并且 **只要流已经开始且上游没有明确报失败，输入 token 也要收**——理由注释写得很清楚：「上游一旦开始生成就已经开始计 prompt 的费用」。
- 失败/不完整状态用 `IsNonBillableResponsesStatus(response.Status)` 判定，明确失败才不计费。

**对我们的映射**：中断时不能简单地"不扣次"或"一律扣 1 次"，而是——**流已开始且非明确失败 → 至少扣 1 次**（按次售卖下的等价规则）；明确失败 → 不扣。这比我先前的草案更站得住脚，且有一手实现依据。

## 六、sub2api：形态与业务差异

- 技术栈：Go 1.27 + Vue 3 + PostgreSQL 15+ + Redis 7+，Docker 部署；后端 `backend/internal/` 下按职责分层：`handler` / `middleware` / `service` / `repository` / `domain` / `model` / `payment` / `platform` / `securityaudit` / `integration` / `setup`。
- 业务：**订阅配额分发**——把 Claude / OpenAI / Gemini / Antigravity 的**订阅账号**额度分发给拼车用户使用。
- **与我们本质不同**：它转售的是**第三方订阅账号的额度**，README 首屏就放着「服务条款风险」「无商业授权」的免责声明；我们转售的是**平台自己付费购买的 API 额度**（ADR-0047 的平台 key）。合规性质不同，这一点值得写进我们自己的产品叙事——**我们卖的是自购额度的转分发，不是账号拼车**。
- 可借鉴：分层与 `payment` 模块化、`securityaudit` 模块的存在本身（把安全审计当一个模块来写）。不借鉴：拼车共享模式与其合规风险面。

## 七、对我们场景的取舍（单上游 + 单客户端 + 按次售卖）

**采纳**：

1. **预扣 + 结算**（用 `status='reserved'` 的用量行承载预留，杜绝并发下窗口超额）；
2. **计费安全不变量**：乘数量上界校验、换算集中一处、饱和事件落审计、新计费路径走全链路核对；
3. **按协议分流的终态集合与 usage 提取器**（配合 `docs/research/deepseek-api-compat.md` 附录 A 的实测形状）；
4. **中转路由同时接受 `Authorization: Bearer` 与 `x-api-key`**；路径同时认 `/v1/...` 与无前缀形态；
5. **令牌模型白名单 + 软删**；
6. **把平台用户 id 透传到上游的 `user` / `user_id` / `metadata.user_id`**，白拿 KVCache 与调度隔离。

**不采纳**（多渠道平台的重型设施，我们是单上游）：

- 渠道池、负载均衡、健康检查、失败换渠道重试、跨分组重试；
- 分组倍率 / 自动分组 / 模型倍率表（我们只需一张套餐表 + 一份阶梯阈值）；
- WebSocket 传输、任务型（图片/视频/异步任务）计费、任务插件体系；
- ClickHouse 日志库、多数据库兼容矩阵、Casbin 授权、多语言 i18n；
- Redis：我们是单实例单上游，限流与并发闸门用进程内状态即可（**若将来水平扩展再引入**，届时并发闸门与窗口预留要挪到共享存储——这一点现在就该记下）。

**存疑待验证**：

- 中断流的估算口径：new-api 用「输出文本 → token 数」估算。我们按次售卖，口径可以更粗（至少 1 次），但如果将来改成按 token 计费，就需要同样的估算能力——**先把 `usage_events` 的字段留全，别到时候改表**。

## 来源清单

| 编号 | 来源 | 说明 |
|---|---|---|
| G1 | `new-api` @ `789c970`，`router/relay-router.go` | 协议面与中间件链、Anthropic 头嗅探（L27/L38） |
| G2 | `new-api` @ `789c970`，`model/token.go` | 令牌表字段设计 |
| G3 | `new-api` @ `789c970`，`service/billing.go`、`service/billing_session.go`、`service/quota.go` | 预扣/结算链路 |
| G4 | `new-api` @ `789c970`，`service/tiered_settle_test.go` | 分段结算与预扣上调 |
| G5 | `new-api` @ `789c970`，`.agents/rules/billing.md` | 计费安全不变量（原文约定，非代码） |
| G6 | `new-api` @ `789c970`，`service/responses_usage.go` | Responses 流式计费累加器、终态集合、中断计费规则 |
| G7 | `new-api` @ `789c970`，`AGENTS.md` | 分层与 `relaykit` 独立模块的定位说明 |
| G8 | `sub2api` @ `0d27f45`，`README.md` / `README_CN.md` / `backend/internal/` 目录 | 形态、技术栈、业务模型与合规提示 |
| G9 | 两个仓库根目录 `LICENSE` | AGPL-3.0 / LGPL-3.0 |

> 配套文档：[deepseek-api-compat.md](deepseek-api-compat.md)（DeepSeek 三个协议面的官方事实 + 本机实测形状）。
> 相关决策：`docs/adr/0047-server-account-package-relay.md`。
