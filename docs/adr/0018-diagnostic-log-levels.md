# 分级诊断日志：五级 + PANIC 标记，flexi_logger 实现

> 更新（2026-08-05）：sidecar 已移除，"绝不写 stdout（RPC 通道）"的约束随之取消——现在 stdout 无协议占用；日志仍走文件 + stderr 双输出，其余结论不变。

在审计（操作事实、不分级）之外，另设分级诊断日志：DEBUG < INFO < WARN < ERROR < CRITICAL（修订：原六档中的 PANIC 不再独立成级，panic hook 以 CRITICAL + [PANIC] 标记记录后退出——既然已 CRITICAL 且即将 panic，单独一档没有操作价值）。日志写入数据根目录 logs/ 的追加式文件，10MB 大小轮转，级别由 settings 配置（log_level）。

实现采用 flexi_logger（实现 log facade，Criterion::Size 轮转），不自研 logger：标准五级与项目五级一一对应，CRITICAL 映射为 ERROR 级 + [CRITICAL] 标记，过滤语义天然正确（log_level=ERROR 时 CRITICAL 一并包含）。kernel 在注册时向插件注入 LoggerHandle（随 PluginContext 提供，包装 log 宏 + 脱敏助手），所有组件统一走同一日志设施；敏感值（API key、令牌）一律经 redact_secret 脱敏，不落日志。Rust panic hook 捕获 panic 上下文写 [PANIC] 后退出（GUI 侧按需提示恢复）。

调研参考 ret2shell（tracing + tracing-subscriber + tracing-appender，按天轮转 + 月/年压缩 + VictoriaLogs 聚合）后的结论：桌面端保留 flexi_logger 大小轮转方案，采纳其两点——文件 + stderr 双输出（WARN 及以上 duplicate_to_stderr；绝不写 stdout，因为 sidecar stdout 是 RPC JSONL 通道）与 panic hook 同时落盘（log::error! + [PANIC] 标记）。
