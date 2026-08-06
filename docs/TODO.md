# TODO

## Agent core 剥离为 so-lite-agent（计划，未落地）

把通用 Agent 运行时（loop/工具注册/会话/模型 Provider 抽象/通用 RPC）剥离为独立 crate `so-lite-agent`，开箱即用（`cargo add` 即可开发新 Agent），内核/用户插件由使用方编写。完整计划见 [docs/plan/so-lite-agent.md](plan/so-lite-agent.md)，决策见 [ADR-0037](adr/0037-so-lite-agent-crate-extraction.md)。当前只做计划，不落地。

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
