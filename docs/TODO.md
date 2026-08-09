# TODO

## 任务书（2026 项目实战·任务 3）覆盖缺口（2026-08-09 对照检查）

对照《2026 项目实战任务书》任务 3（智能学习错题 Agent）五场景与加分项逐条核对后，以下项未完全覆盖：

### 场景 3：多周期学习复盘（缺口最大）

- [ ] **复盘维度不全**：当前只有 `report::weekly`（近 N 天周复盘），任务书要求按 日 / 周 / 单元 / 月考 / 学期 多维度生成报告——缺 daily/unit/monthly/semester 四档（可做成通用 `report::overview` 带 period 参数，weekly 兼容保留）。
- [ ] **统计指标缺**：现只统计总数/正确率/新增错题/最薄弱知识点；任务书还要求「答题时长、正确率变化、持续薄弱考点」——答题时长当前无任何采集点（上传→判分链路未记时），需先埋点再统计。
- [ ] **复习清单导出**：任务书要求自动生成专属复习清单并导出 PDF 或 Markdown——前端无导出入口，后端无清单生成。

### 场景 4：阶段性考核验证

- [ ] **考核闭环不完整**：`exam::compose` 只输出题目 + 答案规格（限时分钟数参数），「限时作答 → 自动判分 → 掌握度判定」未接通——判分可复用 practice::check/grading 管线，需端到端串起来。
- [ ] **达标情况可视化**：知识点「达标 / 待巩固」状态无前端展示（错题本页仅有搜索/排序/置顶/软删除）。

### 场景 5：长效查漏补缺追踪

- [ ] **知识掌握图谱未落地**：任务书要求「节点 = 知识点，边 = 关联关系」的动态知识图谱——当前 `tracking::checkin` 只是手动触发的掌握度快照 + 重测计划（7/14/30 天），非实时图谱；知识点间关联关系无数据模型。
- [ ] **反复丢分考点**：checkin 快照可支撑但无长期跟踪聚合视图（如连续 N 次快照间反复丢分的考点列表）。

### 加分项

- [ ] **错题本导出 Anki 卡组 / PDF**：完全未落地。
- [ ] **知识图谱交互式可视化（力导向图）**：未落地（依赖上面知识图谱数据模型）。
- [ ] **语音提问 + 拍照讲解**：未落地（走现有 vision::read → 讲解管线即可，需 WebView 麦克风权限）。
- [ ] **家长端报表订阅**：未落地（PROJECT.md §14 已列后续优化）。
- [ ] **手写公式 / 图形 OCR 鲁棒识别**：依赖 Qwen3-VL 能力，无专项评测集验证（可建 10~20 张手写卷小评测集评估识别率，作为答辩素材）。

### 交付物缺口（任务书必交）

- [ ] **演示视频**：5 个场景各 1-2 分钟，未产出。
- [ ] **Prompt 人工评测报告**：docs/prompts.md 有 prompt 记录但无正式人工评测报告（任务书要求"人工评测若干题"）。
- [ ] **答辩要点：LangChain/LangGraph 取舍说明**：任务书"强烈建议"LangGraph，本项目为自研 Rust kernel（PROJECT.md §2 有理由），需在技术文档/答辩中明确对比说明。

## Agent core 剥离为 so-lite-agent（计划，未落地）

把通用 Agent 运行时（loop/工具注册/会话/模型 Provider 抽象/通用 RPC）剥离为独立 crate `so-lite-agent`，开箱即用（`cargo add` 即可开发新 Agent），内核/用户插件由使用方编写。完整计划见 [docs/plan/so-lite-agent.md](plan/so-lite-agent.md)，决策见 [ADR-0037](adr/0037-so-lite-agent-crate-extraction.md)。当前只做计划，不落地。

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

现状：AGENTS.md（教学规则，家长/老师可编辑）已完成初始化写入（见上一条），但内核系统提示仍是静态文本（[src/kernel/prompt.rs](../src/kernel/prompt.rs) gent_system_prompt()），文件内容对模型行为暂无影响。

目标：
- gent_system_prompt() 改为加载数据根目录 AGENTS.md 全文进系统提示（PROJECT.md §6 指令加载 / ADR-0011 / ADR-0012）
- 缺失、损坏或超限时回退当前静态文本；路径校验仅限数据根目录内（参照 ootstrap::init_data_root）
- 建议与设置页「教学规则」编辑入口（或「打开规则文件」按钮）配套落地，前端展示规则已加载状态
- 优先级：中（MVP 不阻塞，静态提示词已覆盖核心教学流程）

参考：PROJECT.md §6 指令加载；[docs/adr/0011-single-data-root.md](adr/0011-single-data-root.md)、[docs/adr/0012-no-skill-system-v2.md](adr/0012-no-skill-system-v2.md)。


## 前端工具元数据去硬编码（已完成）

[web/src/lib/tools.js](../web/src/lib/tools.js) 建立工具目录模块：启动时经 `list_tools` 拉取一次并缓存 `entry → {title, icon, group}`，`toolIcon` / `toolTitle` / `toolList` 都从它取；`messages.js` 的 `TOOL_ICONS` / `TOOL_TITLES` 已删除，渲染与 `FORCED_RE` 还原统一走目录（缺失回退 entry 名）。ChatPage 与 SessionsPage 共用该模块，前端不再维护工具名 → 展示信息映射。
