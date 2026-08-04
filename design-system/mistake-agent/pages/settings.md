# 页面覆盖：设置（settings）

基于 MASTER.md，以下为本页专属规则：

- 入口：侧边栏「设置」（mdi:cog-outline），数据经 `get_settings` / `set_settings` RPC。
- 安全：API Key 一律不回显，只显示 `key_set` 状态；输入框留空 = 不修改。
- 分区：通用（日志级别）、主模型（Responses/Chat Completions 切换）、视觉模型（默认 Chat Completions）。
- 保存后显示成功提示，并重新拉取公开视图；错误就近显示在表单上方。
- 表单：标签 + 输入框 + 帮助文案；聚焦态 3px teal 光环；所有控件 ≥44px。
