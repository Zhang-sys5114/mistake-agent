# 页面覆盖：错题本（mistakes）

基于 MASTER.md，以下为本页专属规则：

- 入口：侧边栏「错题本」（mdi:format-list-bulleted），数据经 `trigger_command("grading::list")`。
- 布局：学科筛选 chips（全部/各学科）+ 响应式卡片网格（minmax 320px）。
- 卡片：学科徽章（teal）、知识点徽章、题目（Markdown/KaTeX）、学生作答、参考答案、错因分析、时间。
- 空状态：笔记本图标 + 引导文案；加载态用旋转图标（mdi:loading）。
- 交互：hover 卡片轻浮起（200ms，translateY -2px）。
