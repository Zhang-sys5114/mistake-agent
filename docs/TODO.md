# TODO

## Agent core 剥离为 so-lite-agent（计划，未落地）

把通用 Agent 运行时（loop/工具注册/会话/模型 Provider 抽象/通用 RPC）剥离为独立 crate `so-lite-agent`，开箱即用（`cargo add` 即可开发新 Agent），内核/用户插件由使用方编写。完整计划见 [docs/plan/so-lite-agent.md](plan/so-lite-agent.md)，决策见 [ADR-0037](adr/0037-so-lite-agent-crate-extraction.md)。当前只做计划，不落地。

## OOBE 初始化数据根目录（未完成）

现状：OOBE 首次向导只调用 `set_settings` 保存模型配置（[web/src/components/OobePage.vue](../web/src/components/OobePage.vue)）；数据根目录 `~/Documents/.mistake-agent/` 下的目录与文件由各模块首次使用时懒创建（storage、memory、logger、uploads），`AGENTS.md`（教学规则）目前没有任何初始化逻辑。

目标：首次向导完成时（或 kernel 启动引导阶段）一次性初始化：

- 创建数据根目录及子目录：`sessions/`、`mistakes/`、`memory/`、`audit/`、`logs/`、`uploads/`
- 若 `AGENTS.md` 不存在，写入默认模板（教学规则占位，家长/老师可编辑；已存在则不覆盖）
- 初始化归 kernel 负责（建议新增内核引导逻辑或 RPC，前端不直接写文件系统），与 `set_settings` 保存一同在 OOBE 完成路径触发

参考：PROJECT.md §6 数据与配置；[src/kernel/settings.rs](../src/kernel/settings.rs) `data_root()`。

## OOBE 指引 API key 获取与余额购买（未完成）

现状：OOBE 向导只让用户填写 API key（[web/src/components/OobePage.vue](../web/src/components/OobePage.vue)），没有告诉用户**去哪里申请 key**、**去哪里购买/充值余额**——新用户可能卡在"没有 key"或"有 key 没余额"的步骤。

目标：OOBE 里加入 key 获取与余额指引：

- **key 获取**：分别引导主模型（DeepSeek 开放平台）与视觉模型（SiliconFlow）的注册/创建 API key 入口（平台控制台链接 + 步骤说明），说明实名、开通等前置条件
- **余额购买**：分别给出两个平台的官方充值入口，说明计费方式（DeepSeek 需充值后使用；SiliconFlow 通常有免费额度/免费模型），配「已配置 key」与「余额校验」步骤（复用 `check_balance` RPC 能力）
- 文案面向非技术家长/学生：为什么需要两个 key、去哪申请、去哪买、大概花多少
- **远期**：推出第一方 API 聚合包（会员制）——一个订阅覆盖双模型，OOBE 改为引导注册/登录会员，替代手动配置双 key

参考：OOBE 组件在 [web/src/components/OobePage.vue](../web/src/components/OobePage.vue)；`check_balance` 在 [src/kernel/agent/balance.rs](../src/kernel/agent/balance.rs) / [src/kernel/agent/rpc.rs](../src/kernel/agent/rpc.rs)。

## 前端工具元数据去硬编码（已完成）

[web/src/lib/tools.js](../web/src/lib/tools.js) 建立工具目录模块：启动时经 `list_tools` 拉取一次并缓存 `entry → {title, icon, group}`，`toolIcon` / `toolTitle` / `toolList` 都从它取；`messages.js` 的 `TOOL_ICONS` / `TOOL_TITLES` 已删除，渲染与 `FORCED_RE` 还原统一走目录（缺失回退 entry 名）。ChatPage 与 SessionsPage 共用该模块，前端不再维护工具名 → 展示信息映射。
