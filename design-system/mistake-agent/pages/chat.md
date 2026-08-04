# 页面覆盖：聊天（chat）

基于 MASTER.md（Flat Design，教育 teal #0D9488 + 课程 amber #D97706），以下为本页专属规则：

- 主操作区：消息流（用户右侧 teal 气泡、助手左侧白卡），流式光标 + 打字机增量。
- 思考过程默认折叠（`details`），图标 `mdi:brain`。
- 工具执行状态以底部胶囊呈现，图标来自内核事件（Iconify `mdi:*`）。
- 分支与编辑：用户气泡在回读会话后显示编辑按钮；同一父节点存在多个子消息时显示分支按钮（点击循环切换）。
- 动画：消息入场 280ms 上浮淡入；工具胶囊 200ms 淡入淡出；全部尊重 `prefers-reduced-motion`。
- 触屏（hover:none）：气泡操作按钮常显；桌面端 hover 显示、focus-within 常显。
- 输入区：44px 触控目标，文件选择（mdi:upload）、发送（mdi:send，amber CTA）、停止（mdi:stop-circle）。
