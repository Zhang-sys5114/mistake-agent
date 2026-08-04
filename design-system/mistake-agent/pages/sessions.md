# 页面覆盖：会话历史（sessions）

基于 MASTER.md，以下为本页专属规则：

- 入口：侧边栏「会话」（mdi:history），数据经 `list_sessions` / `read_session` RPC。
- 列表：卡片行展示目标、最后活动时间、状态；点击进入详情。
- 详情：顶部返回按钮 + 目标徽章；消息按时间顺序平铺，按 kind 显示图标
  （user=mdi:account、assistant=mdi:robot-outline、tool_call=mdi:toolbox-outline）。
- 目标字段兼容两种形态：字符串或 `{text}` 对象。
- 空状态：mdi:history + 提示文案；接口未接通时展示明确错误而非空白。
