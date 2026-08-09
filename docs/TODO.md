# TODO

## 任务书（2026 项目实战·任务 3）落地任务（2026-08-09 设计方案已定，决策见 ADR-0039/0040/0041）

### 基础架构改造（三个场景的地基，先做）

- [ ] **错题本目录化 + 事件流**（ADR-0039）：`mistakes/<id>/mistake.json + events.jsonl + schedule.json`；`graded`/`mastery_changed` 事件纯追加（带 subject/knowledge_point 冗余 + duration_seconds 可选）；bootstrap 迁移旧 `mistakes.json`（逐题原子拆 + 幂等 + `.bak` + backfill 事件）。
- [ ] **掌握度调度与裁决**（ADR-0040）：Anki 式 schedule.json（interval/ease/due_at，错 1 次重置 7 天 / 答对 ×2）；连错 2 次打回 is_correct；`grading::update` 升级 UserAndModel Tool 只限内容字段（前端 trigger_command 零改动）；删除/管理字段保持 UserOnly。

### 场景 3：多周期学习复盘

- [ ] `report::weekly` 加 `period` 参数（daily/weekly/monthly/semester；不传=旧行为）；semester 支持 `start_at/end_at` 可选参数（模型会话式问用户学期起止）。
- [ ] 持续薄弱考点：近 N 天错 ≥3 次 且连续两期上榜（硬编码），`weakest_points[]` 加 `persistent` 标记。
- [ ] 答题时长采集：exam 计时器自动记 + 上传批改学生自述（模型填 duration_seconds）；practice::check 不采集；提示词让模型告知用户"作答计时"。
- [ ] 复习清单：report 输出 Markdown，前端「导出」= Blob 下载 .md + window.print() 打印 PDF。
- [ ] ECharts 按需打包 + `ReportChart.vue`（后端出结构化 JSON，前端只渲染）。

### 场景 4：阶段性考核验证

- [ ] `exam::compose` 加 `paper_type`（quiz/unit/midterm/final/gaokao）映射难度配置，复用 practice 出题核心（模板三档 + 真题池 + LLM 兜底）。
- [ ] 限时作答：前端计时器，到点提醒 + 自愿提交 + 真实用时记录（超时如实统计）。
- [ ] 判分：模型逐题调 `practice::check`（不新建批量入口）。
- [ ] 达标判定：卷内该知识点题数 ≥2 且得分率 ≥80% → 自动置 true + `mastery_changed(source=exam_pass)`；前端达标/待巩固可视化（ECharts 上色）。

### 场景 5：长效查漏补缺追踪

- [ ] 每学科 `mistakes/graph/<学科>.json`（sanitize + 路径校验）；先验边（模板依赖表人工标注）+ 共现边（判分批次增量）事件驱动更新；跨学科边存发起学科文件。
- [ ] `tracking::graph_query` 工具（UserAndModel）：知识点 → 掌握度/邻居/关联错题/近期事件（Agentic RAG 落地，不做向量）。
- [ ] 主动重测回合（ADR-0041）：30 分钟 tokio 定时器扫 due → InterruptBus 排队 / 空闲发起 proactive 回合；每知识点每天提醒 ≤1 次；拒绝则 24h 冷却。
- [ ] 反复丢分考点聚合视图：跨快照/跨事件统计「连续两期以上均丢分」的知识点清单（数据源：事件流时间线 + schedule），供 report/tracking 输出与图谱高亮。
- [ ] 知识图谱力导向图：ECharts graph 渲染（数据源 graph.json）。

### 加分项

- [ ] **知识图谱力导向图**：方案已定（ECharts graph + graph.json，实现见场景 5 对应项）。
- [ ] **错题本导出 Anki 卡组**：前端导出 tab 分隔文本（问题\t答案\t知识点标签\t错因），Anki「文件→导入」直接成卡组；PDF 复用复习清单打印。
- [ ] **语音提问**：MediaRecorder 录音 → SiliconFlow `audio/transcriptions`（SenseVoice）→ 文本回填输入框（用户确认后发送）；**拍照讲解**：getUserMedia 进附件管线（vision::read）。
- [ ] **手写 OCR 评测**：🔬 待测——vision::read 功能已覆盖；答辩兜底：用现有 3 套样例（含 1 真实手写）端到端跑通结果整理进 docs/testing.md 作鲁棒性证据，暂不建评测集。
- [ ] **家长端报表订阅**：⏸ 挂起——候选形态为设置页 PIN 家长模式 + 学情总览视图（复用 ReportChart），未排期。

### 交付物缺口（任务书必交）

- [ ] **演示视频**：5 个场景各 1-2 分钟，未产出。
- [ ] **Prompt 人工评测报告**：docs/prompts.md 有 prompt 记录但无正式人工评测报告（任务书要求"人工评测若干题"）。
- [ ] **答辩要点：LangChain/LangGraph 取舍说明**：任务书"强烈建议"LangGraph，本项目为自研 Rust kernel（PROJECT.md §2 有理由），需在技术文档/答辩中明确对比说明。
- [ ] **项目复盘报告**：任务书通用规则 D14 必交 1-2 页（做对了什么/踩了什么坑/学到什么），未产出。
- [ ] **Agent 流程图**：任务书交付物要求"源代码仓库（含 Agent 流程图、Prompt 库）"——Prompt 库已有（docs/prompts.md），缺 agent 工作流图（工具调度/会话切换/重测循环的流程图，答辩文档用）。


## Agent core 剥离为 so-lite-agent（计划，未落地）

把通用 Agent 运行时（loop/工具注册/会话/模型 Provider 抽象/通用 RPC）剥离为独立 crate `so-lite-agent`，开箱即用（`cargo add` 即可开发新 Agent），内核/用户插件由使用方编写。完整计划见 [docs/plan/so-lite-agent.md](plan/so-lite-agent.md)，决策见 [ADR-0037](adr/0037-so-lite-agent-crate-extraction.md)。当前只做计划，不落地。

## 近期：英语练习模式（规划，未落地）

沉浸式英语环境：开启后整个对话环境切全英文，含模型输出。

- settings.json 加 `english_mode: bool`（用户独占写，默认 false；设置页开关）。
- 启动/热更新时生效：`agent_system_prompt()`（[src/kernel/prompt.rs](../src/kernel/prompt.rs)）在 english_mode 下替换为英文版系统提示（或追加强指令"All replies must be in English"），全链路模型输出（含判分/出题/复盘）随主系统提示走英文。
- 范围决策：判定模型指令（判分、摘要等）是否也切英文——倾向跟随（同一沉浸语境）；GUI 界面文字暂不切（只切模型对话侧，UI 留中文更安全）。
- 提示词让模型在 english_mode 下判分/讲解也用英文（练习 + 答题一体）。

## 近期：桌面输入方式增强（规划，未落地）

- **剪贴板粘贴截图**：WebView 监听 `paste`（Ctrl+V / 右键粘贴），图片直接进入附件暂存，与「选择作业文件」共用 vision__read → 判分归档管线。
- **摄像头拍题**：调用 WebView `getUserMedia` 拍题入队，拍完即走同一条 OCR 管线；需处理 WebView2 相机权限与设备选择。

## 中期：Android 手机 / 平板适配（规划，未落地）

- Tauri v2 增加 Android target：移动端壳、触控/窄屏响应式适配、相册/摄像头/剪贴板输入、Pyodide 在移动 WebView 的可用性与性能验证、移动端存储路径与权限模型、离线包体积控制。构建不依赖 macOS（Windows 装 Android SDK 即可）。

## 长期：iOS / iPadOS 适配（规划，未落地）

- 在 Android 落地后追加 iOS/iPadOS target：Apple 相机/相册/剪贴板权限、平台差异收敛到统一能力层。
- **本机无 macOS 的解法**：构建/签名/发布走云 macOS——优先 GitHub Actions macOS runner（本仓库公开，macOS 构建免费额度），签名证书与描述文件以仓库 secrets 托管，CI 出 ipa 后上传 App Store Connect；备选 Codemagic / MacStadium 云 Mac。

## OOBE 初始化数据根目录（已完成）

[src/kernel/bootstrap.rs](../src/kernel/bootstrap.rs) 的 `init_data_root` 在 `Kernel::new` 引导阶段与 `set_settings` 保存路径中执行（幂等）：创建数据根目录及 `sessions/ mistakes/ memory/ audit/ logs/ uploads/` 六个子目录；`AGENTS.md` 缺失时写入默认教学规则模板（存在不覆盖）。storage/logger/memory 各自的懒创建已收敛到 bootstrap。


## AGENTS.md 加载进系统提示（未完成）

现状：AGENTS.md（教学规则，家长/老师可编辑）已完成初始化写入（见上一条），但内核系统提示仍是静态文本（[src/kernel/prompt.rs](../src/kernel/prompt.rs) agent_system_prompt()），文件内容对模型行为暂无影响。

目标：
- agent_system_prompt() 改为加载数据根目录 AGENTS.md 全文进系统提示（PROJECT.md §6 指令加载 / ADR-0011 / ADR-0012）
- 缺失、损坏或超限时回退当前静态文本；路径校验仅限数据根目录内（参照 bootstrap::init_data_root）
- 建议与设置页「教学规则」编辑入口（或「打开规则文件」按钮）配套落地，前端展示规则已加载状态
- 优先级：中（MVP 不阻塞，静态提示词已覆盖核心教学流程）

参考：PROJECT.md §6 指令加载；[docs/adr/0011-single-data-root.md](adr/0011-single-data-root.md)、[docs/adr/0012-no-skill-system-v2.md](adr/0012-no-skill-system-v2.md)。


## 前端工具元数据去硬编码（已完成）

[web/src/lib/tools.js](../web/src/lib/tools.js) 建立工具目录模块：启动时经 `list_tools` 拉取一次并缓存 `entry → {title, icon, group}`，`toolIcon` / `toolTitle` / `toolList` 都从它取；`messages.js` 的 `TOOL_ICONS` / `TOOL_TITLES` 已删除，渲染与 `FORCED_RE` 还原统一走目录（缺失回退 entry 名）。ChatPage 与 SessionsPage 共用该模块，前端不再维护工具名 → 展示信息映射。
