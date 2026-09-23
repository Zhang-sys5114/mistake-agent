# 页面覆盖：会话列表（session-list，应用侧栏内）

基于 MASTER.md。原「会话历史」独立页已删除（ADR-0044 收尾）——会话列表不再是导航项，
也不再是聊天页里的独立一列，而是**应用侧栏 `.sidebar` 内、`会话列表` 开关图标正下方的展开区**
（[SessionListPanel.vue](../../../web/src/components/SessionListPanel.vue)，挂点 `.sidebar-sessions`），
数据经 `list_sessions` / `read_session` / `create_session` / `open_session` / `rename_session` / `delete_session` RPC。
本文件对该区域覆盖 [chat.md](chat.md)。

- 面板头：标题 + 「新对话」按钮（mdi:plus，主色）；回合在飞（busy）时按钮禁用，避免与半截回答抢会话。
- 列表行：标题（单行省略）+ 相对时间副行；当前会话高亮（左侧主色条 + 底色）。按 `last_activity_at` 倒序。
- 标题回退链：`title` → `goal.text` → 「新会话」；缺字段不得渲染空白行。
- 行内操作：重命名（内联输入，形态同消息气泡的 `.edit-inline`）、删除（二次确认弹窗，形态同错题本
  `.confirm-overlay` / `.confirm-dialog.card`，主按钮 `.btn ghost`、危险按钮 `.btn danger`）。
- 会话边界气泡：迁移后每条会话可能以「上一会话梗概」system 消息开头，渲染为可折叠分隔气泡
  （`<details>`，图标 mdi:content-cut），默认折叠——它是背景，不是对话内容。
- 空状态：mdi:history + 提示文案；接口未接通时展示明确错误而非空白。
- 折叠：开关在**侧栏「设置」正下方**（`.nav-item` + `mdi:history`，开启态 `.panel-toggle-on` 弱底），
  列表展开在**同一列的下方**，不另起一列。展开时侧栏按 `.expanded.session-open` 加宽到 288px
  （否则标题放不下）；面板头部另有 `mdi:chevron-left` 快捷收起。
- 状态归属：`App.vue` 持有 `sessionPanelOpen` 与 `activeSessionKey`（列表是应用级 chrome，
  哪个页面都看得见）；点会话若不在聊天页会切回聊天页，因为会话内容在聊天区。
- 动画：面板收起/展开 200ms；全部尊重 `prefers-reduced-motion`。
- 触屏（hover:none）：行内操作按钮常显；桌面端 hover 显示、focus-within 常显。
