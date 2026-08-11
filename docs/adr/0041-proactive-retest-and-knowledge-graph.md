# 主动重测回合与知识图谱

决策：两个机制，均为场景 5（长效追踪）的落地形态。机制层（scheduler 内核插件、磁盘 IO 铁律、数据运行时化）见 ADR-0042。

## 主动重测回合

- **触发**：tracking 插件经 SchedulerHandle 注册定时配置（30 分钟周期 + 提醒载荷文本 + `fire_on_start` 启动即触发）；scheduler 内核插件只存配置、到点**请求 kernel 核心**，由内核特权在回合边界发起 Interrupt（"环境有变动"信号）——用户态只发系统调用，用户插件永不触碰中断。
- **回合**：中断在回合边界排队、不并入用户回合；回合空闲时 kernel 核心从 pending proactive 队列消费，发起**独立 proactive 回合**（无用户新消息）。模型输入 = 提醒载荷 + 环境变动提示；模型经白名单工具 `tracking::due_list` 自查到期清单（防骚扰过滤折叠在工具内部），产出**合成 user 消息**（proactive 标记 + display_text 前端渲染为系统通知气泡）入当前聊天树（无活跃会话时建专属提醒会话）；学生回复即为该节点子节点，重测在普通回合接续。
- **边界**：proactive 回合工具白名单**缺省为空**（tracking 显式传 `["tracking::due_list"]`），kernel 硬护栏（白名单只缩不扩 + 全局频率上限）——结构性杜绝模型在提醒回合自动出题；到期后的动作链（`practice::generate` → `practice::check` → 事件 → 调度更新）只走学生回应后的普通回合。
- **防骚扰**：`last_reminded_at` 与 `dismissed_until` **纯内存**（重启作废）：运行期内每知识点最多提醒 1 次；学生拒绝经 UserAndModel 工具 `tracking::dismiss` 记 24h 冷却；App 就绪即触发一次（fire_on_start），保证"每次打开提醒一次积压到期"。

## 知识图谱

每学科一个 `mistakes/graph/<学科>.json`（sanitize + 路径校验；storage 持有文件、tracking 持有语义，用户插件读写全经 StorageHandle——磁盘 IO 铁律）。图谱**纯拓扑**（节点 id + 边 + 权重），属性（掌握度/关联错题/近期事件）经 schedule.json/events.jsonl **实时聚合**，可重建。

- 节点全局 ID = `学科::知识点`；图谱**按学科隔离**（无跨学科边），前缀仅作撞名消歧。
- **共现层**：一次 grading 判分调用 = 一个批次（graded 事件带 `batch_id`），同批知识点两两成边，**权重 = 共现批次数**（批内去重，同批 N 题只让每对边 +1）；**双层剪枝**：写入宽松（weight ≥ 1 即入图文件，保全拓扑）、查询严格（graph_query 默认只返强关联，弱边探索模式才返）；**无时间衰减**。
- **先验层**：启动时由模型生成一次前置依赖表（`A→B` = A 是 B 的前置）→ 落盘 `data/point_deps.json` → 固化读取（**生成即数据**，ADR-0042）；不开放运行时人工维护。
- **LLM 抽取层**：列后续，不设计不排期。

## 工具面

- `tracking::due_list`（UserAndModel）：扫 due_at + 防骚扰过滤，返回"该提醒"的到期清单。
- `tracking::dismiss`（UserAndModel）：记 24h 内存冷却。
- `tracking::graph_query`（UserAndModel）：输入 学科 + 知识点 → 输出 `mastery` / `neighbors`（含前置方向）/ `related_mistakes` / `recent_events`——给原料不给结论（解释是模型的工作）。
- `tracking::graph`（UserOnly Command）：前端经 trigger_command 拉全图拓扑（节点 + 边 + 权重）供 ECharts 力导向图渲染。

动机：无提醒学生必忘（纯被动 checkin 不可靠）；自动重测既不必要（模型出题能力已足够、学生必须在场作答）也破坏机制/策略分层；图谱是错题数据的投影，真相在事件流。

考虑过的替代方案：纯被动 checkin 扫描（被否：无提醒学生必忘）；前端横幅常驻通知（被否：改动大且本地工具无推送场景）；图数据库（被否：违反本地单二进制红线，见 ADR-0041 原版否决项）；向量语义检索（被否：结构化精确过滤优先，列后续）；跨学科边（被否：共现层天然同科、先验层无跨学科先例）。
